use std::path::PathBuf;
use std::time::Duration;

use dekan_core::state::{InjectionStatus, StateSender, set_injection_status};
use tracing::{error, info, warn};

use crate::dll_validator::validate_dll_hash;
use crate::error::InjectError;
use crate::overlay::{OverlayConfig, OverlayManager};
use crate::overlay_process::OverlayProcess;
use crate::suspend::SuspendGuard;

pub const DEFAULT_BUILD_TIMEOUT: Duration = Duration::from_secs(300);

pub const HOOK_CONFIRMED_STATUS: &str = "Waiting for exit";

pub const HOOK_PATCHING_STATUS: &str = "Patching";

pub const PATCHER_ARMED_STATUS: &str = "Waiting for league match to start";

pub const DEFAULT_ARM_TIMEOUT: Duration = Duration::from_secs(10);

pub const DEFAULT_HOOK_TIMEOUT: Duration = Duration::from_secs(40);

pub const DEFAULT_MAX_SUSPENSION: Duration = Duration::from_secs(60);

pub const MAX_SUSPENSION_LIMIT: Duration = Duration::from_secs(180);

struct CancelOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub ltk_host_exe: PathBuf,

    pub ltk_host_hash: String,

    pub ltk_dll_path: PathBuf,

    pub ltk_dll_hash: String,

    pub ltk_flags: u32,

    pub overlay_config: OverlayConfig,

    pub state_dir: PathBuf,

    pub hook_timeout: Duration,

    pub build_timeout: Duration,

    pub max_suspension: Duration,
}

pub struct InjectionOutcome {
    pub status: InjectionStatus,

    pub overlay: Option<OverlayProcess>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayBuild {
    pub wad_files: usize,

    pub bytes: u64,

    pub elapsed: Duration,
}

fn measure_overlay(overlay_dir: &std::path::Path) -> (usize, u64) {
    fn walk(dir: &std::path::Path, files: &mut usize, bytes: &mut u64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => walk(&path, files, bytes),
                Ok(ft) if ft.is_file() => {
                    let is_wad = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".wad.client"));
                    if is_wad {
                        *files += 1;
                        *bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                    }
                }
                _ => {}
            }
        }
    }

    let mut files = 0usize;
    let mut bytes = 0u64;
    walk(overlay_dir, &mut files, &mut bytes);
    (files, bytes)
}

pub struct InjectionPipeline {
    config: PipelineConfig,
    state_tx: Option<StateSender>,
}

impl InjectionPipeline {
    #[must_use]
    pub fn new(config: PipelineConfig, state_tx: Option<StateSender>) -> Self {
        Self { config, state_tx }
    }

    fn publish(&self, status: InjectionStatus) {
        if let Some(ref tx) = self.state_tx {
            set_injection_status(tx, status);
        }
    }

    fn validate_patcher_binaries(&self) -> Result<(), InjectError> {
        validate_dll_hash(&self.config.ltk_host_exe, &self.config.ltk_host_hash)?;
        validate_dll_hash(&self.config.ltk_dll_path, &self.config.ltk_dll_hash)
    }

    pub async fn execute(
        &self,
        mods: &[String],
        game_pid: u32,
        game_tid: u32,
    ) -> Result<InjectionOutcome, InjectError> {
        self.publish(InjectionStatus::Pending);

        info!(
            pid = game_pid,
            tid = game_tid,
            mods = ?mods,
            "Starting injection pipeline"
        );

        if let Err(e) = self.validate_patcher_binaries() {
            error!(error = %e, "Injection aborted: DLL hash mismatch or missing");
            self.publish(InjectionStatus::Failed {
                error: format!("DLL validation error: {e}"),
            });
            return Err(e);
        }

        let suspend_guard = match SuspendGuard::acquire(
            game_pid,
            game_tid,
            Some(&self.config.state_dir),
        ) {
            Ok(guard) => Some(guard),
            Err(e) => {
                warn!(
                    pid = game_pid,
                    tid = game_tid,
                    error = %e,
                    "Could not suspend game thread (access denied or protected); proceeding with injection without suspension"
                );
                None
            }
        };

        let suspended_at = std::time::Instant::now();
        let mut suspend_guard = suspend_guard;
        let result = self
            .run_with_game_suspended(mods, suspended_at, &mut suspend_guard)
            .await;
        if suspend_guard.is_some() {
            info!(
                suspended_ms = suspended_at.elapsed().as_millis(),
                "Game suspension window closed"
            );
        }

        match result {
            Ok((status, overlay)) => {
                if let Some(guard) = suspend_guard {
                    guard.resume()?;
                }
                self.publish(status.clone());
                Ok(InjectionOutcome {
                    status,
                    overlay: Some(overlay),
                })
            }
            Err(e) => {
                if let Some(guard) = suspend_guard {
                    if let Err(resume_err) = guard.resume() {
                        error!(error = %resume_err, "EMERGENCY: failed to resume game after injection error");
                    }
                }
                error!(error = %e, "Injection failed");
                self.publish(InjectionStatus::Failed {
                    error: e.to_string(),
                });
                Err(e)
            }
        }
    }

    async fn run_with_game_suspended(
        &self,
        mods: &[String],
        suspended_at: std::time::Instant,
        suspend_guard: &mut Option<SuspendGuard>,
    ) -> Result<(InjectionStatus, OverlayProcess), InjectError> {
        let budget = self.config.max_suspension.min(MAX_SUSPENSION_LIMIT);
        let remaining = |now: &std::time::Instant| budget.saturating_sub(now.elapsed());

        let build_timeout = self.config.build_timeout.min(remaining(&suspended_at));
        self.build_overlay(mods, build_timeout).await?;

        let mut overlay = self.spawn_patcher().await?;

        if let Some(guard) = suspend_guard.take() {
            if let Err(e) = guard.resume() {
                error!(error = %e, "Failed to resume game after building the overlay");
                return Err(e);
            }
            info!(
                suspended_ms = suspended_at.elapsed().as_millis(),
                "Game suspension window closed; process resumed so the patcher can hook it"
            );
        }

        let hook_budget = self.config.hook_timeout.min(remaining(&suspended_at));
        let status = self.confirm_hook(&mut overlay, hook_budget).await;

        Ok((status, overlay))
    }

    pub async fn arm(
        &self,
        mods: &[String],
    ) -> Result<(OverlayProcess, OverlayBuild), InjectError> {
        self.publish(InjectionStatus::Pending);

        info!(mods = ?mods, "Arming the patcher before the game starts");

        if let Err(e) = self.validate_patcher_binaries() {
            error!(error = %e, "Arming aborted: DLL hash mismatch or missing");
            self.publish(InjectionStatus::Failed {
                error: format!("DLL validation error: {e}"),
            });
            return Err(e);
        }

        let build = match self.build_overlay(mods, self.config.build_timeout).await {
            Ok(build) => build,
            Err(e) => {
                error!(error = %e, "Arming aborted: the overlay could not be built");
                self.publish(InjectionStatus::Failed {
                    error: e.to_string(),
                });
                return Err(e);
            }
        };

        let mut overlay = match self.spawn_patcher().await {
            Ok(overlay) => overlay,
            Err(e) => {
                error!(error = %e, "Arming aborted: the patcher could not be started");
                self.publish(InjectionStatus::Failed {
                    error: e.to_string(),
                });
                return Err(e);
            }
        };

        match overlay
            .wait_for_line(
                |line| line.contains(PATCHER_ARMED_STATUS),
                DEFAULT_ARM_TIMEOUT,
            )
            .await
        {
            Ok(_) => {
                info!(
                    wad_files = build.wad_files,
                    overlay_bytes = build.bytes,
                    build_ms = build.elapsed.as_millis(),
                    "Patcher armed and watching for the game"
                );
                Ok((overlay, build))
            }
            Err(e) => {
                let exit = overlay.exited_within(Duration::from_millis(500)).await;
                error!(
                    error = %e,
                    exit_code = ?exit,
                    "The patcher never reported that it is watching for the game; the skin will not load"
                );
                self.publish(InjectionStatus::Failed {
                    error: format!("patcher failed to arm: {e}"),
                });
                Err(e)
            }
        }
    }

    async fn build_overlay(
        &self,
        mods: &[String],
        timeout: Duration,
    ) -> Result<OverlayBuild, InjectError> {
        OverlayManager::prepare_overlay_dir(&self.config.overlay_config.overlay_dir)?;

        let effective_game_dir =
            dekan_platform::paths::normalize_game_dir(&self.config.overlay_config.game_dir)
                .unwrap_or_else(|| self.config.overlay_config.game_dir.clone());

        if let Some((wad_files, bytes)) = crate::overlay_cache::OverlayCache::is_fresh(
            &effective_game_dir,
            &self.config.overlay_config.mods_dir,
            &self.config.overlay_config.overlay_dir,
            mods,
        ) {
            info!(
                mods = ?mods,
                wad_files,
                overlay_bytes = bytes,
                "Overlay cache hit: fingerprint and base game WADs unchanged; skipping build"
            );
            return Ok(OverlayBuild {
                wad_files,
                bytes,
                elapsed: Duration::ZERO,
            });
        }

        match self.build_overlay_native(mods, timeout).await {
            Ok(build) => {
                self.record_overlay_cache(&effective_game_dir, mods);
                Ok(build)
            }
            Err(e) => {
                crate::overlay_cache::OverlayCache::invalidate(
                    &self.config.overlay_config.overlay_dir,
                );
                if !matches!(e, InjectError::Cancelled) {
                    warn!(error = %e, mods = ?mods, budget_ms = timeout.as_millis(), "Native overlay build failed");
                }
                Err(e)
            }
        }
    }

    fn record_overlay_cache(&self, game_dir: &std::path::Path, mods: &[String]) {
        if let Err(e) = crate::overlay_cache::OverlayCache::record(
            game_dir,
            &self.config.overlay_config.mods_dir,
            &self.config.overlay_config.overlay_dir,
            mods,
        ) {
            warn!(
                error = %e,
                "Overlay cache fingerprint not recorded; the next match rebuilds this overlay"
            );
        }
    }

    async fn build_overlay_native(
        &self,
        mods: &[String],
        timeout: Duration,
    ) -> Result<OverlayBuild, InjectError> {
        let config = &self.config.overlay_config;
        let game_dir = dekan_platform::paths::normalize_game_dir(&config.game_dir)
            .unwrap_or_else(|| config.game_dir.clone());
        let (mods_dir, overlay_dir) = (config.mods_dir.clone(), config.overlay_dir.clone());
        let mods = mods.to_vec();

        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _cancel_on_drop = CancelOnDrop(cancel.clone());
        let flag = cancel.clone();
        let task = tokio::task::spawn_blocking(move || {
            crate::overlay_builder::build(&game_dir, &mods_dir, &overlay_dir, &mods, &flag)
        });
        let native = match tokio::time::timeout(timeout, task).await {
            Ok(Ok(result)) => result?,
            Ok(Err(join)) => {
                return Err(InjectError::Overlay(format!(
                    "native overlay task failed: {join}"
                )));
            }
            Err(_) => {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                return Err(InjectError::SubprocessTimeout {
                    command: "overlay build".into(),
                    timeout_secs: timeout.as_secs(),
                });
            }
        };

        let (wad_files, bytes) = measure_overlay(&self.config.overlay_config.overlay_dir);
        if wad_files == 0 {
            return Err(InjectError::Overlay(
                "the overlay build produced an empty overlay".into(),
            ));
        }
        info!(
            wad_files,
            rewritten = native.written,
            overlay_bytes = bytes,
            elapsed_ms = native.elapsed.as_millis(),
            "Overlay built natively"
        );
        Ok(OverlayBuild {
            wad_files,
            bytes,
            elapsed: native.elapsed,
        })
    }

    async fn spawn_patcher(&self) -> Result<OverlayProcess, InjectError> {
        OverlayProcess::spawn_ltk_host(
            &self.config.ltk_host_exe,
            &self.config.overlay_config.overlay_dir,
            self.config.ltk_flags,
            crate::ltk_host::HostLogLevel::from_env(),
        )
        .await
    }

    pub async fn confirm_hook(
        &self,
        overlay: &mut OverlayProcess,
        budget: Duration,
    ) -> InjectionStatus {
        let waited = std::time::Instant::now();

        if budget.is_zero() {
            warn!("Suspension budget already spent; resuming without waiting for the hook");
            return InjectionStatus::Unconfirmed;
        }

        let result = overlay
            .wait_for_line(|line| line.contains(HOOK_CONFIRMED_STATUS), budget)
            .await;

        match result {
            Ok(line) => {
                info!(
                    status_line = %line.text,
                    elapsed_ms = waited.elapsed().as_millis(),
                    "Hook confirmed by the patcher before resuming the game"
                );
                InjectionStatus::Confirmed
            }
            Err(e) => {
                if let Some(code) = overlay.exited_within(Duration::from_millis(500)).await {
                    warn!(
                        exit_code = code,
                        "The patcher exited before confirming the hook; the overlay is not active"
                    );
                    return InjectionStatus::Failed {
                        error: format!("the patcher exited with code {code} before hooking"),
                    };
                }

                warn!(
                    error = %e,
                    elapsed_ms = waited.elapsed().as_millis(),
                    "Hook not confirmed; resuming the game and reporting Unconfirmed"
                );
                InjectionStatus::Unconfirmed
            }
        }
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
