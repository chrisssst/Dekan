use tracing::{debug, info, warn};

use crate::boot::panel::tray_status;

pub fn spawn_custom_mods_notice(
    supervisor: &mut dekan_core::supervisor::Supervisor,
    notice_tray: dekan_platform::tray::TrayController,
    custom_mods_rx: tokio::sync::oneshot::Receiver<dekan_app::mod_repair::ScanSummary>,
) {
    supervisor.spawn("custom-mods-notice", move |child_token| async move {
        let received = tokio::select! {
            _ = child_token.cancelled() => return,
            received = custom_mods_rx => received,
        };
        let Ok(summary) = received else {
            debug!("The custom mods scan ended without a report");
            return;
        };
        let text = dekan_platform::i18n::text();
        let mut lines = Vec::new();
        if summary.repaired > 0 {
            lines.push(dekan_platform::i18n::fill(
                text.custom_mods_repaired_body,
                "n",
                &summary.repaired.to_string(),
            ));
        }
        if summary.refused > 0 {
            lines.push(dekan_platform::i18n::fill(
                text.custom_mods_refused_body,
                "n",
                &summary.refused.to_string(),
            ));
        }
        if !lines.is_empty() {
            notice_tray.notify(text.custom_mods_repaired_title, &lines.join(" "));
        }
    });
}

pub fn spawn_update_check(
    supervisor: &mut dekan_core::supervisor::Supervisor,
    offer: crate::boot::update::UpdateOffer,
    update_notice: &dekan_app::update_check::UpdateNotice,
) {
    let state_rx_update = offer.state_rx.clone();
    let check = dekan_app::update_check::UpdateCheck {
        state_dir: offer.state_dir.clone(),
        notice: update_notice.clone(),
        notify: Box::new(move |latest| Box::pin(crate::boot::update::offer(offer.clone(), latest))),
    };
    supervisor.spawn("update-check", move |child_token| {
        dekan_app::update_check::run(check, state_rx_update, child_token)
    });
}

pub fn spawn_tray_status(
    supervisor: &mut dekan_core::supervisor::Supervisor,
    state_rx: &dekan_core::state::StateReceiver,
    tray_controller: dekan_platform::tray::TrayController,
    tool_files: Vec<std::path::PathBuf>,
) {
    let mut state_rx_tray = state_rx.clone();
    let text = dekan_platform::i18n::text();
    supervisor.spawn_essential("tray-status-updater", move |child_token| async move {
        loop {
            tokio::select! {
                _ = child_token.cancelled() => break,
                res = state_rx_tray.changed() => {
                    if res.is_err() {
                        break;
                    }

                    let tools_missing = !tool_files.iter().all(|file| file.is_file());
                    let state = state_rx_tray.borrow();
                    tray_controller.update_status(tray_status(&state, tools_missing, text));
                    let (party_line, in_room) = dekan_app::control_panel::party_line(&state.shown_party_status(), state.party_hosting, text);
                    tray_controller.update_party(&party_line, in_room);
                }
            }
        }
    });
}

pub fn spawn_activation_listener(
    supervisor: &mut dekan_core::supervisor::Supervisor,
    activation_panel: Option<dekan_platform::panel::PanelLinks>,
) {
    match dekan_platform::activation::ActivationListener::create(crate::INSTANCE_NAME) {
        Ok(listener) => {
            supervisor.spawn_essential("activation-listener", move |child_token| async move {
                loop {
                    tokio::select! {
                        _ = child_token.cancelled() => break,
                        () = tokio::time::sleep(tokio::time::Duration::from_millis(400)) => {
                            if listener.take_request() {
                                info!("Another launch was detected; surfacing this instance");
                                match &activation_panel {
                                    Some(links) => dekan_platform::panel::show_panel(links.clone()),
                                    None => dekan_platform::welcome::show_welcome_window(),
                                }
                            }
                        }
                    }
                }
                drop(listener);
            });
        }

        Err(e) => {
            warn!(error = %e, "Could not create the activation listener");
        }
    }
}

pub fn spawn_log_drop_monitor(
    supervisor: &mut dekan_core::supervisor::Supervisor,
    dropped_lines: tracing_appender::non_blocking::ErrorCounter,
) {
    supervisor.spawn_essential("log-drop-monitor", move |child_token| async move {
        let mut reported = 0usize;
        loop {
            tokio::select! {
                _ = child_token.cancelled() => break,
                () = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                    let now = dropped_lines.dropped_lines();
                    if now > reported {
                        warn!(
                            dropped_lines = now,
                            since_last_report = now - reported,
                            "Log writer queue overflowed; lines were dropped"
                        );
                        reported = now;
                    }
                }
            }
        }
    });
}
