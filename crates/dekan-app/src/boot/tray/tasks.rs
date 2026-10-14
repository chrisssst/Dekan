use tracing::{info, warn};

use crate::boot::injector::install_from_panel;
use crate::boot::panel::tray_status;

pub fn spawn_custom_mods_notice(
    supervisor: &mut dekan_core::supervisor::Supervisor,
    notice_tray: dekan_platform::tray::TrayController,
    custom_mods_rx: std::sync::mpsc::Receiver<dekan_app::mod_repair::ScanSummary>,
) {
    supervisor.spawn("custom-mods-notice", move |child_token| async move {
        let received = tokio::select! {
            _ = child_token.cancelled() => return,
            received = tokio::task::spawn_blocking(move || custom_mods_rx.recv()) => received,
        };
        let Ok(Ok(summary)) = received else {
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

pub fn spawn_injector_install(
    supervisor: &mut dekan_core::supervisor::Supervisor,
    ltk_notice: &dekan_app::ltk_release::LtkNotice,
    tools_dir: &std::path::Path,
    state_dir: &std::path::Path,
    tray: &dekan_platform::tray::TrayController,
) -> tokio::sync::mpsc::UnboundedSender<()> {
    let (install_tx, mut install_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let install_tray = tray.clone();
    let install_notice = ltk_notice.clone();
    let install_tools = tools_dir.to_path_buf();
    let install_state = state_dir.to_path_buf();
    supervisor.spawn("injector-install", move |child_token| async move {
        loop {
            tokio::select! {
                _ = child_token.cancelled() => break,
                request = install_rx.recv() => {
                    if request.is_none() {
                        break;
                    }
                    install_from_panel(&install_notice, &install_tools, &install_state, &install_tray).await;
                    while install_rx.try_recv().is_ok() {}
                }
            }
        }
    });
    install_tx
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

pub fn spawn_ltk_check(
    supervisor: &mut dekan_core::supervisor::Supervisor,
    tray_controller: &dekan_platform::tray::TrayController,
    state_dir_path: &std::path::Path,
    ltk_dll_path: &std::path::Path,
    installed_dll: &dekan_app::ltk_release::InstalledDllCache,
    ltk_notice: &dekan_app::ltk_release::LtkNotice,
    state_rx: &dekan_core::state::StateReceiver,
) {
    let ltk_tray = tray_controller.clone();
    let ltk_check = dekan_app::ltk_release::LtkCheck {
        state_dir: state_dir_path.to_path_buf(),
        installed_dll: ltk_dll_path.to_path_buf(),
        installed: installed_dll.clone(),
        notice: ltk_notice.clone(),
        notify: Box::new(move |version| {
            let text = dekan_platform::i18n::text();
            ltk_tray.notify(
                &dekan_platform::i18n::fill(text.ltk_new_title, "version", version),
                text.ltk_new_body,
            );
        }),
    };
    let state_rx_ltk = state_rx.clone();
    supervisor.spawn("ltk-check", move |child_token| {
        dekan_app::ltk_release::run(ltk_check, state_rx_ltk, child_token)
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
    supervisor.spawn("tray-status-updater", move |child_token| async move {
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
                    let (party_line, in_room) = dekan_app::control_panel::party_line(&state.party_status, state.party_hosting, text);
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
            supervisor.spawn("activation-listener", move |child_token| async move {
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
