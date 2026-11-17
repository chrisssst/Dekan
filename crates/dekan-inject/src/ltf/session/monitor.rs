use std::time::Duration;

use dekan_core::state::{InjectionStatus, StateSender, set_injection_status};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, warn};

use crate::ltf::client::{BridgeClient, extract_patcher_state};
use crate::ltf::log_tail::{ManagerLogEvent, ManagerLogTail};
use crate::overlay_process::PatcherSignal;

const LOG_POLL_INTERVAL: Duration = Duration::from_millis(150);
const STATUS_POLL_EVERY_N_TICKS: u32 = 5;

pub(super) struct MonitorContext {
    pub client: BridgeClient,
    pub tx: mpsc::UnboundedSender<PatcherSignal>,
    pub log_tail: ManagerLogTail,
    pub state_tx: Option<StateSender>,
    pub stop_rx: oneshot::Receiver<()>,
}

pub(super) fn spawn_monitor(ctx: MonitorContext) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let MonitorContext {
            client,
            tx,
            mut log_tail,
            state_tx,
            mut stop_rx,
        } = ctx;

        let mut seen_running = false;
        let mut hook_sent = false;
        let mut post_hook_rejected = false;
        let mut last_error: Option<String> = None;
        let mut tick: u32 = 0;

        loop {
            tokio::select! {
                _ = &mut stop_rx => {
                    break;
                }
                _ = tokio::time::sleep(LOG_POLL_INTERVAL) => {}
            }

            let mut stop_now = false;
            for event in log_tail.poll_events() {
                match event {
                    ManagerLogEvent::OverlayBuildComplete(line) => {
                        info!(manager_log = %line, "Mod Manager overlay build completed");
                    }
                    ManagerLogEvent::ScanningStarted(line) => {
                        seen_running = true;
                        info!(manager_log = %line, "Mod Manager host scanning for League window");
                    }
                    ManagerLogEvent::GameFound { tid, pid, line } => {
                        seen_running = true;
                        info!(tid = ?tid, pid = ?pid, manager_log = %line, "Mod Manager found League window and installed hook");
                    }
                    ManagerLogEvent::DllAttached { pid, line } => {
                        seen_running = true;
                        info!(pid = ?pid, manager_log = %line, "Mod Manager patcher DLL attached to League of Legends");
                        if !hook_sent {
                            let _ = tx.send(PatcherSignal::Hooked); // ignore-ok: receiver may have already proceeded
                            hook_sent = true;
                        }
                    }
                    ManagerLogEvent::DllInitDone(line) => {
                        seen_running = true;
                        info!(manager_log = %line, "Mod Manager patcher DLL hooks initialized inside game process");
                        if !hook_sent {
                            let _ = tx.send(PatcherSignal::Hooked); // ignore-ok: receiver may have already proceeded
                            hook_sent = true;
                        }
                    }
                    ManagerLogEvent::RedirectedWad(line) => {
                        info!(manager_log = %line, "Mod Manager patcher redirected WAD read");
                    }
                    ManagerLogEvent::JoinedTooLate(line) => {
                        warn!(manager_log = %line, "Mod Manager patcher joined League too late; overlay was not applied");
                        let msg = format!("Mod Manager patcher joined the game too late: {line}");
                        last_error = Some(msg.clone());
                        report_failure(
                            &tx,
                            &state_tx,
                            &mut hook_sent,
                            &mut post_hook_rejected,
                            msg,
                        );
                        stop_now = true;
                    }
                    ManagerLogEvent::VerificationWarning(line) => {
                        warn!(manager_log = %line, "Mod Manager patcher reported a non-fatal verification warning");
                    }
                    ManagerLogEvent::HookFailed(line)
                    | ManagerLogEvent::VerificationFailed(line)
                    | ManagerLogEvent::ElevationMismatch(line)
                    | ManagerLogEvent::StartFailed(line) => {
                        error!(manager_log = %line, "Mod Manager patcher reported a fatal hook or integrity error");
                        let msg = line.clone();
                        last_error = Some(msg.clone());
                        report_failure(
                            &tx,
                            &state_tx,
                            &mut hook_sent,
                            &mut post_hook_rejected,
                            msg,
                        );
                        stop_now = true;
                    }
                    ManagerLogEvent::PatcherStopped(line) => {
                        info!(manager_log = %line, "Mod Manager patcher stopped");
                        if !hook_sent {
                            let _ = tx.send(PatcherSignal::Failed); // ignore-ok: report early exit before attach
                        }
                        stop_now = true;
                    }
                }
            }

            if stop_now {
                break;
            }

            if (!hook_sent || !seen_running) && tick % STATUS_POLL_EVERY_N_TICKS == 0 {
                if let Ok(status) = client.patcher_status().await {
                    let (is_running, attached, _) = extract_patcher_state(&status);
                    if is_running {
                        seen_running = true;
                    }
                    if attached && !hook_sent {
                        let _ = tx.send(PatcherSignal::Hooked); // ignore-ok: receiver may have already proceeded
                        hook_sent = true;
                    }
                    if seen_running && !is_running && !hook_sent {
                        warn!(
                            reason = ?last_error,
                            "Mod Manager patcher stopped before attaching to League"
                        );
                        let _ = tx.send(PatcherSignal::Failed); // ignore-ok: report failure
                        break;
                    }
                }
            }

            tick = tick.wrapping_add(1);
        }
    })
}

fn report_failure(
    tx: &mpsc::UnboundedSender<PatcherSignal>,
    state_tx: &Option<StateSender>,
    hook_sent: &mut bool,
    post_hook_rejected: &mut bool,
    message: String,
) {
    if !*hook_sent {
        let _ = tx.send(PatcherSignal::Failed); // ignore-ok: report failure to pipeline
        *hook_sent = true;
    } else if !*post_hook_rejected {
        *post_hook_rejected = true;
        if let Some(ref sender) = *state_tx {
            set_injection_status(sender, InjectionStatus::Failed { error: message });
        }
    }
}
