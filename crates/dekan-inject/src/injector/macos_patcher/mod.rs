use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use dekan_core::state::{InjectionStatus, StateSender, set_injection_status};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::error::InjectError;
use crate::overlay::OverlayManager;
use crate::overlay_process::{OverlayProcess, PatcherSignal};

pub mod macho;
pub mod payload;
pub mod sys;

pub use macho::CpuArch;

pub const MACOS_OVERLAY_SYMLINK: &str = "/tmp/dekan_ovl";

pub fn prepare_macos_overlay_prefix(overlay_dir: &Path) -> Result<String, InjectError> {
    std::fs::create_dir_all(overlay_dir)?;
    #[cfg(unix)]
    {
        let link_path = Path::new(MACOS_OVERLAY_SYMLINK);
        if link_path.exists() || link_path.symlink_metadata().is_ok() {
            let _ = std::fs::remove_file(link_path); // ignore-ok: replace previous symlink
        }
        if std::os::unix::fs::symlink(overlay_dir, link_path).is_ok() {
            return Ok(format!("{MACOS_OVERLAY_SYMLINK}/"));
        }
    }
    let mut raw = overlay_dir.display().to_string();
    if !raw.ends_with('/') {
        raw.push('/');
    }
    if raw.len() >= payload::MAX_PREFIX_BYTES {
        return Err(InjectError::Overlay(format!(
            "overlay prefix '{}' exceeds {} bytes",
            raw,
            payload::MAX_PREFIX_BYTES
        )));
    }
    Ok(raw)
}

pub fn patch_game_pid(pid: u32, prefix: &str) -> Result<CpuArch, InjectError> {
    sys::patch_live_process(pid, prefix).map_err(InjectError::Process)
}

pub fn run_cli_patcher(args: &[String]) -> Result<Option<i32>, String> {
    let Some(idx) = args.iter().position(|a| a == "--macos-patcher") else {
        return Ok(None);
    };
    let prefix = args
        .get(idx + 1)
        .ok_or_else(|| "--macos-patcher requires an overlay prefix path".to_owned())?;
    let pid = match args.iter().position(|a| a == "--pid") {
        Some(p_idx) => args
            .get(p_idx + 1)
            .ok_or_else(|| "--pid requires a numeric process ID".to_owned())?
            .parse::<u32>()
            .map_err(|e| format!("invalid --pid value: {e}"))?,
        None => dekan_platform::process::ProcessFinder::find_any_process(
            &dekan_platform::game_version::GAME_EXES,
        )
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "LeagueofLegends process not running".to_owned())?,
    };
    match patch_game_pid(pid, prefix) {
        Ok(arch) => {
            println!("MACPATCH_HOOKED pid={pid} arch={arch:?}");
            Ok(Some(0))
        }
        Err(e) => {
            eprintln!("MACPATCH_FAILED pid={pid} error={e}");
            Ok(Some(1))
        }
    }
}

fn patch_pid_elevated_or_direct(pid: u32, prefix: &str) -> Result<(), String> {
    if dekan_platform::elevation::is_elevated() {
        patch_game_pid(pid, prefix)
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
        let args = format!("--macos-patcher \"{prefix}\" --pid {pid}");
        match dekan_platform::elevation::run_elevated(&exe, &args) {
            Ok(dekan_platform::elevation::ElevatedRun::Finished(0)) => Ok(()),
            Ok(dekan_platform::elevation::ElevatedRun::Finished(code)) => {
                Err(format!("elevated macOS patcher exited with code {code}"))
            }
            Ok(dekan_platform::elevation::ElevatedRun::Declined) => {
                Err("macOS administrator authorization was declined".into())
            }
            Err(e) => Err(format!("elevated macOS patcher failed: {e}")),
        }
    }
}

pub async fn spawn_macos_patcher(
    mods_dir: &Path,
    mods: &[String],
    state_tx: Option<StateSender>,
) -> Result<OverlayProcess, InjectError> {
    let data_dir =
        dekan_platform::paths::data_dir().unwrap_or_else(|_| std::env::temp_dir().join("Dekan"));
    let overlay_dir = data_dir.join("overlay");
    let game_dir = dekan_platform::paths::discover_game_dir().ok_or_else(|| {
        InjectError::Overlay("League of Legends game directory not found on macOS".into())
    })?;

    OverlayManager::prepare_overlay_dir(&overlay_dir)?;
    let fresh =
        crate::overlay_cache::OverlayCache::is_fresh(&game_dir, mods_dir, &overlay_dir, mods);
    if fresh.is_none() {
        let g_dir = game_dir.clone();
        let m_dir = mods_dir.to_path_buf();
        let o_dir = overlay_dir.clone();
        let mod_names = mods.to_vec();
        let cancel = AtomicBool::new(false);
        tokio::task::spawn_blocking(move || {
            crate::overlay_builder::build(&g_dir, &m_dir, &o_dir, &mod_names, &cancel)
        })
        .await
        .map_err(|e| InjectError::Overlay(format!("macOS overlay build task failed: {e}")))??;
        let _ = crate::overlay_cache::OverlayCache::record(&game_dir, mods_dir, &overlay_dir, mods); // ignore-ok: cache recording is best-effort
    }

    let prefix = prepare_macos_overlay_prefix(&overlay_dir)?;
    info!(
        prefix = %prefix,
        overlay = %overlay_dir.display(),
        mods = ?mods,
        "macOS Mach-O patcher armed with native WAD overlay"
    );

    let (tx, rx) = mpsc::unbounded_channel();
    let _ = tx.send(PatcherSignal::Armed); // ignore-ok: receiver is open
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();

    let monitor = tokio::spawn(async move {
        let mut last_patched_pid: Option<u32> = None;
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                () = tokio::time::sleep(Duration::from_millis(120)) => {
                    let found_pid = dekan_platform::process::ProcessFinder::find_any_process(
                        &dekan_platform::game_version::GAME_EXES,
                    )
                    .ok()
                    .flatten();

                    match found_pid {
                        Some(pid) if last_patched_pid != Some(pid) => {
                            let prefix_owned = prefix.clone();
                            let res = tokio::task::spawn_blocking(move || {
                                patch_pid_elevated_or_direct(pid, &prefix_owned)
                            })
                            .await;
                            match res {
                                Ok(Ok(())) => {
                                    last_patched_pid = Some(pid);
                                    info!(pid, "macOS Mach-O patch applied to LeagueofLegends");
                                    let _ = tx.send(PatcherSignal::Hooked); // ignore-ok: notify hook confirmed
                                    if let Some(ref stx) = state_tx {
                                        set_injection_status(stx, InjectionStatus::Confirmed);
                                    }
                                }
                                Ok(Err(err)) => {
                                    warn!(pid, error = %err, "macOS Mach-O patch failed");
                                    let _ = tx.send(PatcherSignal::Failed); // ignore-ok: notify hook failure
                                    if let Some(ref stx) = state_tx {
                                        set_injection_status(stx, InjectionStatus::Failed { error: err });
                                    }
                                    break;
                                }
                                Err(join_err) => {
                                    warn!(pid, error = %join_err, "macOS Mach-O patch task panicked");
                                    let _ = tx.send(PatcherSignal::Failed); // ignore-ok: notify hook failure
                                    break;
                                }
                            }
                        }
                        Some(_) => {}
                        None => {
                            last_patched_pid = None;
                        }
                    }
                }
            }
        }
    });

    let stop_slot = Arc::new(std::sync::Mutex::new(Some(stop_tx)));
    let shutdown_hook: crate::overlay_process::ShutdownHook = Box::new(move || {
        Box::pin(async move {
            if let Ok(mut guard) = stop_slot.lock()
                && let Some(s) = guard.take()
            {
                let _ = s.send(()); // ignore-ok: stop macOS patcher loop
            }
        })
    });

    Ok(OverlayProcess {
        child: None,
        _stdin: None,
        signals: rx,
        _stdout_reader: Some(monitor),
        _stderr_reader: None,
        hooked: false,
        shutdown_hook: Some(shutdown_hook),
    })
}

#[cfg(test)]
mod tests;
