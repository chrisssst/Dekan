use std::path::PathBuf;
use std::time::Duration;

use dekan_core::state::{InjectionStatus, StateSender, set_injection_status};
use tracing::{error, info, warn};

use crate::error::InjectError;
use crate::overlay::{OverlayConfig, OverlayManager};
use crate::overlay_process::{OverlayProcess, PatcherSignal};

pub const DEFAULT_BUILD_TIMEOUT: Duration = Duration::from_secs(300);

pub const SAFE_HOOK_WINDOW: Duration = Duration::from_secs(2);

const LOADING_GAME_POLL: Duration = Duration::from_millis(500);

#[must_use]
pub fn game_already_loading() -> Option<(u32, Duration)> {
    let pid = dekan_platform::process::ProcessFinder::find_any_process(
        &dekan_platform::game_version::GAME_EXES,
    )
    .ok()
    .flatten()?;
    let age = dekan_platform::process::ProcessFinder::process_age(pid)?;
    (age > SAFE_HOOK_WINDOW).then_some((pid, age))
}

async fn wait_out_loading_game() {
    let mut reported = false;
    while let Some((pid, age)) = game_already_loading() {
        if !reported {
            reported = true;
            warn!(
                pid,
                age_ms = age.as_millis(),
                "The game was already loading when the patcher became ready; it is not hooked mid-load. The skin loads when the game starts again (reconnect)"
            );
        }
        tokio::time::sleep(LOADING_GAME_POLL).await;
    }
}

pub const DEFAULT_ARM_TIMEOUT: Duration = Duration::from_secs(10);

pub const DEFAULT_HOOK_TIMEOUT: Duration = Duration::from_secs(40);

pub const DEFAULT_LATE_BUDGET: Duration = Duration::from_secs(60);

pub const LATE_BUDGET_LIMIT: Duration = Duration::from_secs(180);

struct CancelOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub ltk_host_exe: PathBuf,

    pub ltk_dll_path: PathBuf,

    pub ltk_flags: u32,

    pub overlay_config: OverlayConfig,

    pub hook_timeout: Duration,

    pub build_timeout: Duration,

    pub late_budget: Duration,
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
        crate::trust::verify_injector_file(&self.config.ltk_host_exe)?;
        crate::trust::verify_injector_file(&self.config.ltk_dll_path)
    }

    pub async fn execute(
        &self,
        mods: &[String],
        game_pid: u32,
    ) -> Result<InjectionOutcome, InjectError> {
        self.publish(InjectionStatus::Pending);

        info!(
            pid = game_pid,
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

        let started = std::time::Instant::now();
        match self.run_late(mods, started).await {
            Ok((status, overlay)) => {
                self.publish(status.clone());
                Ok(InjectionOutcome {
                    status,
                    overlay: Some(overlay),
                })
            }
            Err(e) => {
                error!(error = %e, "Injection failed");
                self.publish(InjectionStatus::Failed {
                    error: e.to_string(),
                });
                Err(e)
            }
        }
    }

    async fn run_late(
        &self,
        mods: &[String],
        started: std::time::Instant,
    ) -> Result<(InjectionStatus, OverlayProcess), InjectError> {
        let budget = self.config.late_budget.min(LATE_BUDGET_LIMIT);
        let remaining = |now: &std::time::Instant| budget.saturating_sub(now.elapsed());

        let build_timeout = self.config.build_timeout.min(remaining(&started));
        self.build_overlay(mods, build_timeout).await?;

        if let Some((_, age)) = game_already_loading() {
            return Err(InjectError::GameAlreadyLoading {
                age_ms: u64::try_from(age.as_millis()).unwrap_or(u64::MAX),
            });
        }

        let mut overlay = self.spawn_patcher().await?;

        let hook_budget = self.config.hook_timeout.min(remaining(&started));
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

        wait_out_loading_game().await;

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
            .wait_for(PatcherSignal::Armed, DEFAULT_ARM_TIMEOUT)
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
            warn!("Late-path budget already spent; not waiting for the hook");
            return InjectionStatus::Unconfirmed;
        }

        let result = overlay.wait_for(PatcherSignal::Hooked, budget).await;

        match result {
            Ok(()) => {
                info!(
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

mod overlay;

#[cfg(test)]
mod tests;
