use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub fn tray_status(
    state: &dekan_core::state::AppState,
    tools_missing: bool,
    text: &'static dekan_platform::i18n::Text,
) -> &'static str {
    use dekan_core::phase::GamePhase;
    use dekan_core::state::InjectionStatus;

    if tools_missing {
        return text.status_tools_missing;
    }
    if !state.lcu_connected {
        return text.status_waiting_league;
    }
    match state.phase {
        GamePhase::Lobby => text.status_lobby,
        GamePhase::Matchmaking | GamePhase::CheckedIntoTournament => text.status_matchmaking,
        GamePhase::ReadyCheck => text.status_ready_check,
        GamePhase::ChampSelect => text.status_champ_select,
        GamePhase::Finalization => text.status_finalization,

        GamePhase::GameStart | GamePhase::InProgress => match state.injection {
            InjectionStatus::Pending => text.status_injecting,
            InjectionStatus::Confirmed => text.status_in_game_confirmed,
            InjectionStatus::Unconfirmed => text.status_in_game_unconfirmed,
            InjectionStatus::Failed { .. } => text.status_in_game_failed,
            InjectionStatus::Idle => text.status_in_game,
        },
        GamePhase::Reconnect => text.status_reconnecting,
        GamePhase::None
        | GamePhase::WaitingForStats
        | GamePhase::PreEndOfGame
        | GamePhase::EndOfGame
        | GamePhase::FailedToLaunch
        | GamePhase::TerminatedInError => text.status_connected,
    }
}

pub struct ReleaseNotices {
    pub dekan: dekan_app::update_check::UpdateNotice,
    pub ltk: dekan_app::ltk_release::LtkNotice,
    pub installed: InstalledInjector,
    pub custom_mods: dekan_app::mod_repair::CustomModsNotice,
}

pub struct InstalledInjector {
    pub dll: std::path::PathBuf,
    pub cache: dekan_app::ltk_release::InstalledDllCache,
}

pub fn panel_links_for(
    tray: &dekan_platform::tray::TrayController,
    state_rx: dekan_core::state::StateReceiver,
    tools: Vec<std::path::PathBuf>,
    game_dir: std::path::PathBuf,
    game_build: Option<u32>,
    elevated: bool,
    notices: ReleaseNotices,
) -> dekan_platform::panel::PanelLinks {
    let controller = tray.clone();
    let snapshot = move || {
        let (party_line, in_room) = controller.party();
        let lcu_connected = state_rx.borrow().lcu_connected;
        let autostart = match dekan_platform::autostart::is_enabled() {
            Ok(enabled) => enabled,
            Err(e) => {
                debug!(error = %e, "Start with Windows setting unreadable; shown as off");
                false
            }
        };
        let facts = dekan_app::control_panel::Facts {
            status: controller.status(),
            party_line,
            in_room,
            auto_accept: dekan_platform::preferences::AUTO_ACCEPT.is_enabled(),
            random_skin: dekan_platform::preferences::RANDOM_SKIN.is_enabled(),
            light_loading: dekan_platform::preferences::LIGHT_LOADING.is_enabled(),
            autostart,
            tools_present: dekan_app::control_panel::tools_present(&tools),
            game_found: dekan_app::control_panel::game_found(&game_dir),
            lcu_connected,
            game_build,
            now_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            elevated,
            update: notices.dekan.available().map(|latest| latest.to_string()),
            ltk: notices.ltk.status(),
            installed_dll: notices.installed.cache.read(&notices.installed.dll),
            custom_mods: notices.custom_mods.summary(),
        };
        dekan_app::control_panel::snapshot(&facts, dekan_platform::i18n::text())
    };
    dekan_platform::panel::PanelLinks {
        events: tray.events(),
        snapshot: std::sync::Arc::new(snapshot),
    }
}

pub struct TrayActions {
    pub shutdown: CancellationToken,
    pub logs_dir: Option<std::path::PathBuf>,
    pub tools_dir: std::path::PathBuf,
    pub mods_dir: std::path::PathBuf,
    pub mod_roots: Vec<std::path::PathBuf>,
    pub state_dir: std::path::PathBuf,
    pub notifier: dekan_platform::tray::TrayController,
    pub party_tx: tokio::sync::mpsc::UnboundedSender<dekan_app::party_manager::PartyCommand>,
    pub links: dekan_platform::panel::PanelLinks,
    pub mark_state: dekan_core::state::StateReceiver,
    pub mark_live: dekan_app::live_game::LiveGame,
    pub install_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

impl TrayActions {
    pub fn handle(&self, event: dekan_platform::tray::TrayEvent) {
        match event {
            dekan_platform::tray::TrayEvent::Quit => {
                info!("Shutdown requested via system tray");
                self.shutdown.cancel();
            }
            dekan_platform::tray::TrayEvent::PartyCreate => {
                // ignore-ok: the manager is gone only when the app is shutting down
                let _ = self
                    .party_tx
                    .send(dekan_app::party_manager::PartyCommand::Create);
            }
            dekan_platform::tray::TrayEvent::PartyJoin => {
                // ignore-ok: the manager is gone only when the app is shutting down
                let _ = self
                    .party_tx
                    .send(dekan_app::party_manager::PartyCommand::Join);
            }
            dekan_platform::tray::TrayEvent::PartyLeave => {
                // ignore-ok: the manager is gone only when the app is shutting down
                let _ = self
                    .party_tx
                    .send(dekan_app::party_manager::PartyCommand::Leave);
            }
            dekan_platform::tray::TrayEvent::OpenMods => {
                let _ = std::fs::create_dir_all(&self.mods_dir); // ignore-ok: create dir if missing before opening
                if let Err(e) = dekan_platform::shell::open_folder(&self.mods_dir) {
                    warn!(error = %e, mods = %self.mods_dir.display(), "Could not open the custom mods folder");
                }
            }
            dekan_platform::tray::TrayEvent::RestoreMods => {
                let text = dekan_platform::i18n::text();
                let outcome =
                    dekan_app::mod_repair::restore_originals(&self.mod_roots, &self.state_dir);
                let body = if outcome.busy {
                    text.custom_mods_restore_busy.to_owned()
                } else if let Some(first) = outcome.failed.first() {
                    warn!(failed = ?outcome.failed, restored = outcome.restored, "Some custom mods could not be restored to their originals");
                    dekan_platform::i18n::fill(text.custom_mods_restore_failed, "error", first)
                } else {
                    dekan_platform::i18n::fill(
                        text.custom_mods_restored_body,
                        "n",
                        &outcome.restored.to_string(),
                    )
                };
                self.notifier.notify(text.custom_mods_restored_title, &body);
            }
            dekan_platform::tray::TrayEvent::Activated => {
                dekan_platform::panel::show_panel(self.links.clone());
            }
            dekan_platform::tray::TrayEvent::ToggleRandomSkin => {
                match dekan_platform::preferences::RANDOM_SKIN.toggle() {
                    Ok(enabled) => info!(
                        enabled,
                        "Random skin when none is chosen changed from the control panel"
                    ),
                    Err(e) => warn!(error = %e, "Could not change the random skin setting"),
                }
            }
            dekan_platform::tray::TrayEvent::ToggleLightLoading => {
                match dekan_platform::preferences::LIGHT_LOADING.toggle() {
                    Ok(enabled) => info!(
                        enabled,
                        "Light match loading changed from the control panel"
                    ),
                    Err(e) => warn!(error = %e, "Could not change the light match loading setting"),
                }
            }
            dekan_platform::tray::TrayEvent::OpenLogs => {
                match self.logs_dir {
                    Some(ref dir) => {
                        let _ = std::fs::create_dir_all(dir); // ignore-ok: open_folder below refuses a missing folder and that refusal is logged
                        if let Err(e) = dekan_platform::shell::open_folder(dir) {
                            warn!(error = %e, "Could not open the logs folder");
                        }
                    }
                    None => warn!(
                        "Logs folder requested from the tray, but its path could not be resolved at boot"
                    ),
                }
            }
            dekan_platform::tray::TrayEvent::OpenTools => {
                let _ = std::fs::create_dir_all(&self.tools_dir); // ignore-ok: create dir if missing before opening
                if let Err(e) = dekan_platform::shell::open_folder(&self.tools_dir) {
                    warn!(error = %e, tools = %self.tools_dir.display(), "Could not open the tools folder");
                }
            }
            dekan_platform::tray::TrayEvent::About => {
                dekan_platform::welcome::show_about_window();
            }
            dekan_platform::tray::TrayEvent::MarkProblem => {
                let phase = self.mark_state.borrow().phase;
                match self.mark_live.latest() {
                    Some(live) => warn!(
                        phase = ?phase,
                        game_time = %self.mark_live
                            .game_time_at(std::time::SystemTime::now())
                            .map(dekan_app::live_game::format_game_time)
                            .unwrap_or_default(),
                        champion = %live.champion,
                        skin_id = live.skin_id,
                        skin_name = %live.skin_name,
                        "User marked a problem"
                    ),
                    None => {
                        warn!(phase = ?phase, "User marked a problem (no live game data at this moment)")
                    }
                }
            }
            dekan_platform::tray::TrayEvent::ExportDiagnostics => {
                match self.logs_dir.as_deref().map(|dir| {
                    dekan_app::control_panel::export_diagnostics(
                        dir,
                        std::time::SystemTime::now(),
                        &[],
                    )
                }) {
                    Some(Ok(zip)) => {
                        info!(file = %zip.display(), "Diagnostics exported");
                        if let Some(dir) = zip.parent() {
                            if let Err(e) = dekan_platform::shell::open_folder(dir) {
                                warn!(error = %e, "Could not open the diagnostics folder");
                            }
                        }
                    }
                    Some(Err(e)) => warn!(error = %e, "Diagnostics could not be exported"),
                    None => warn!(
                        "Diagnostics requested, but the logs folder could not be resolved at boot"
                    ),
                }
            }
            dekan_platform::tray::TrayEvent::OpenRelease => {
                let page = dekan_app::update_check::release_page();
                if let Err(e) = dekan_platform::shell::open_web_page(&page) {
                    warn!(error = %e, page = %page, "Could not open the release page");
                }
            }
            dekan_platform::tray::TrayEvent::InstallInjector => {
                // ignore-ok: the install task is gone only when the app is shutting down
                let _ = self.install_tx.send(());
            }
            dekan_platform::tray::TrayEvent::ToggleAutostart => {
                match dekan_platform::autostart::toggle() {
                    Ok(enabled) => info!(enabled, "Start with Windows changed from the tray"),
                    Err(e) => warn!(error = %e, "Could not change the Start with Windows setting"),
                }
            }
            dekan_platform::tray::TrayEvent::ToggleAutoAccept => {
                match dekan_platform::preferences::AUTO_ACCEPT.toggle() {
                    Ok(enabled) => info!(enabled, "Automatic match accept changed from the tray"),
                    Err(e) => {
                        warn!(error = %e, "Could not change the automatic match accept setting")
                    }
                }
            }
        }
    }
}
