#![windows_subsystem = "windows"]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use anyhow::Result;
use dekan_inject::suspend::{OrphanRecovery, recover_orphaned_suspension};
use dekan_platform::paths::state_dir;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use dekan_app::catalog;

mod logging;
mod trigger;

const INSTANCE_NAME: &str = "dekan";

#[tokio::main]
async fn main() -> Result<()> {
    let state_dir_path = state_dir().unwrap_or_else(|_| std::env::temp_dir().join("Dekan_state"));

    let mut lock_failure = None;
    let instance_guard = match dekan_platform::single_instance::SingleInstanceGuard::acquire(
        INSTANCE_NAME,
        Some(&state_dir_path),
    ) {
        Ok(guard) => Some(guard),
        Err(dekan_platform::error::PlatformError::AlreadyRunning { .. }) => {
            let _ = dekan_platform::activation::request_activation(INSTANCE_NAME); // ignore-ok: the notice below is shown either way
            dekan_platform::activation::notify_already_running();
            return Ok(());
        }

        Err(e) => {
            lock_failure = Some(e);
            None
        }
    };

    let logs_dir_path = dekan_platform::paths::logs_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("Dekan").join("logs"));
    let logging = logging::init(&logs_dir_path)?;
    let elevated = dekan_platform::elevation::is_elevated();

    let debug_privilege = if elevated {
        dekan_platform::elevation::enable_debug_privilege()
    } else {
        false
    };
    info!(
        version = env!("CARGO_PKG_VERSION"),
        logs_dir = %logs_dir_path.display(),
        single_instance = instance_guard.is_some(),
        elevated,
        debug_privilege,
        "Dekan starting"
    );

    match dekan_platform::user_profile::resolution() {
        dekan_platform::user_profile::Resolution::SameUser { path } => {
            debug!(local_app_data = %path.display(), "Running as the desktop user");
        }
        dekan_platform::user_profile::Resolution::DesktopUser { path, own } => info!(
            local_app_data = %path.display(),
            own_profile = ?own,
            "Running as another account than the desktop user; data goes to the desktop user's profile"
        ),
        dekan_platform::user_profile::Resolution::OwnProfile { path, reason } => warn!(
            local_app_data = %path.display(),
            reason = %reason,
            "Desktop user could not be determined; using this process's own profile"
        ),
        dekan_platform::user_profile::Resolution::Unresolved { reason } => {
            error!(reason = %reason, "No LocalAppData could be resolved");
        }
    }
    if !elevated {
        info!(
            "Dekan is running without administrator privileges: the late-injection fallback \
             cannot suspend the game"
        );
    }
    if let Some(e) = lock_failure {
        warn!(error = %e, "Single-instance lock unavailable; a second launch will not be blocked");
    }

    match recover_orphaned_suspension(&state_dir_path, dekan_platform::game_version::GAME_EXE) {
        OrphanRecovery::Clean => {}
        OrphanRecovery::Corrupt { content } => {
            warn!(content = %content, "Suspension sentinel was malformed; removed")
        }
        OrphanRecovery::Gone { pid } => info!(
            pid,
            "The game an earlier run left suspended is no longer running; nothing to resume"
        ),
        OrphanRecovery::NotTheGame { pid, exe } => warn!(
            pid,
            exe = %exe,
            "The PID an earlier run suspended now belongs to another program; left untouched"
        ),
        OrphanRecovery::Resumed { pid, tid } => {
            warn!(pid, tid, "Resumed the game an earlier run left suspended")
        }
        OrphanRecovery::Failed { pid, tid, error } => error!(
            pid,
            tid,
            error = %error,
            "The game an earlier run left suspended could not be resumed; close it from Task Manager"
        ),
    }

    let (state_tx, state_rx) = dekan_core::state::new_state_channel();

    let shutdown_token = CancellationToken::new();
    let mut supervisor = dekan_core::supervisor::Supervisor::new(shutdown_token.clone());

    remove_retired_bridge_files(&state_dir_path);

    let lcu_observer = dekan_lcu::observer::LcuObserver::new(state_tx.clone(), None);
    supervisor.spawn("lcu-observer", move |child_token| async move {
        lcu_observer.run(child_token).await;
    });

    let dropped_lines = logging.dropped.clone();
    supervisor.spawn("log-drop-monitor", move |child_token| async move {
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

    let (party_tx, party_rx) =
        tokio::sync::mpsc::unbounded_channel::<dekan_app::party_manager::PartyCommand>();
    {
        let manager = dekan_app::party_manager::PartyManager::new(
            state_tx.clone(),
            state_rx.clone(),
            state_dir_path.clone(),
        );
        supervisor.spawn("party-manager", move |child_token| async move {
            manager.run(child_token, party_rx).await;
        });
    }

    let mut paths = trigger::ResolvedPaths::discover();
    paths.library_dir = catalog::resolve_library_root(&paths.library_dir);
    let mut library_root = paths.library_dir.clone();
    if let Err(e) = std::fs::create_dir_all(&library_root) {
        warn!(path = %library_root.display(), error = %e, "Could not create library root directory");
    }

    let local_library = dekan_platform::paths::data_dir()
        .map(|d| d.join("library"))
        .unwrap_or_else(|_| library_root.clone());

    let dir_has_content = |dir: &std::path::Path| -> bool {
        dir.is_dir()
            && std::fs::read_dir(dir)
                .map(|mut entries| entries.any(|e| e.is_ok()))
                .unwrap_or(false)
    };

    let mut library_has_content = dir_has_content(&library_root);
    if !library_has_content || (!dir_has_content(&local_library) && library_root != local_library) {
        let fallback_candidates = [
            paths.tools_dir.parent().map(|p| p.join("library")),
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|p| p.join("library"))),
            if library_has_content {
                Some(library_root.clone())
            } else {
                None
            },
        ];
        if let Some(source) = fallback_candidates
            .into_iter()
            .flatten()
            .find(|p| dir_has_content(p))
        {
            if source != local_library {
                info!(
                    source = %source.display(),
                    target = %local_library.display(),
                    "Seeding skin library into LocalAppData"
                );
                seed_directory(&source, &local_library);
            }
            if dir_has_content(&local_library) {
                paths.library_dir = local_library.clone();
                library_root = local_library;
                library_has_content = true;
            } else {
                warn!(
                    fallback = %source.display(),
                    "Could not seed LocalAppData library; falling back to read-only library source"
                );
                paths.library_dir = source.clone();
                library_root = source;
                library_has_content = true;
            }
        }
    }

    if paths.game_dir.is_dir() {
        dekan_inject::overlay_builder::prewarm_game_index(&paths.game_dir);
    }

    if library_has_content {
        info!(library = %library_root.display(), "Skin library resolved");
    } else {
        warn!(
            library = %library_root.display(),
            "Skin library is empty or missing; official skins will be generated from the installed game"
        );
    }

    if let Some(sync_config) = dekan_app::skin_sync::SkinSyncConfig::from_env_value(
        std::env::var(dekan_core::env::SKIN_SYNC).ok().as_deref(),
    ) {
        info!(
            library = %library_root.display(),
            source = %sync_config.zip_url,
            "Skin library download enabled (DEKAN_SKIN_SYNC)"
        );
        let sync_lib_dir = library_root.clone();
        supervisor.spawn("skin-sync", move |child_token| async move {
            tokio::select! {
                _ = child_token.cancelled() => {}
                res = dekan_app::skin_sync::sync_skin_library(
                    &sync_lib_dir,
                    &sync_config,
                    false,
                ) => {
                    match res {
                        Ok(dekan_app::skin_sync::SkinSyncResult::Updated { sha, files_extracted }) => {
                            info!(
                                sha = %sha,
                                files = files_extracted,
                                "Skin library synchronized and updated from upstream repository"
                            );
                        }
                        Ok(dekan_app::skin_sync::SkinSyncResult::UpToDate { sha }) => {
                            debug!(sha = %sha, "Skin library is up to date with upstream repository");
                        }
                        Ok(dekan_app::skin_sync::SkinSyncResult::Skipped { reason }) => {
                            debug!(reason = %reason, "Skin library synchronization skipped");
                        }
                        Err(e) => {
                            warn!(error = %e, "Skin library sync failed; existing skins remain functional");
                        }
                    }
                }
            }
        });
    }

    report_game_build(&state_dir_path, &paths.game_dir, &paths.overlay_dir);

    let required_tools = trigger::required_tool_files(&paths);

    let text = dekan_platform::i18n::text();
    let initial_status = if paths.tools_source == trigger::ToolsSource::Missing {
        text.status_tools_missing
    } else {
        text.status_waiting_league
    };

    let tray = match dekan_platform::tray::SystemTray::spawn(initial_status) {
        Ok(tray) => Some(tray),
        Err(e) => {
            error!(error = %e, "Could not create the system tray icon; Dekan has no visible UI and can only be closed from the Task Manager");
            None
        }
    };
    if let Some(tray) = tray {
        let tray_shutdown = shutdown_token.clone();
        let tray_logs_dir = dekan_platform::paths::logs_dir().ok();
        let tray_tools_dir = paths.tools_dir.clone();
        let tray_mods_dir = paths.custom_mods_root.clone();
        let tray_controller = tray.controller();

        dekan_platform::welcome::show_welcome_window();

        supervisor.spawn("tray-events", move |child_token| async move {
            loop {
                tokio::select! {
                    _ = child_token.cancelled() => break,
                    () = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                        while let Some(event) = tray.try_recv_event() {
                            match event {
                                dekan_platform::tray::TrayEvent::Quit => {
                                    info!("Shutdown requested via system tray");
                                    tray_shutdown.cancel();
                                }
                                dekan_platform::tray::TrayEvent::PartyCreate => {

                                    // ignore-ok: the manager is gone only when the app is shutting down
                                    let _ = party_tx.send(dekan_app::party_manager::PartyCommand::Create);
                                }
                                dekan_platform::tray::TrayEvent::PartyJoin => {

                                    // ignore-ok: the manager is gone only when the app is shutting down
                                    let _ = party_tx.send(dekan_app::party_manager::PartyCommand::Join);
                                }
                                dekan_platform::tray::TrayEvent::PartyLeave => {

                                    // ignore-ok: the manager is gone only when the app is shutting down
                                    let _ = party_tx.send(dekan_app::party_manager::PartyCommand::Leave);
                                }
                                dekan_platform::tray::TrayEvent::OpenMods => {
                                    let _ = std::fs::create_dir_all(&tray_mods_dir); // ignore-ok: create dir if missing before opening
                                    if let Err(e) = dekan_platform::shell::open_folder(&tray_mods_dir) {
                                        warn!(error = %e, mods = %tray_mods_dir.display(), "Could not open the custom mods folder");
                                    }
                                }
                                dekan_platform::tray::TrayEvent::OpenLogs | dekan_platform::tray::TrayEvent::Activated => {
                                    match tray_logs_dir {
                                        Some(ref dir) => {
                                            let _ = std::fs::create_dir_all(dir); // ignore-ok: open_folder below refuses a missing folder and that refusal is logged
                                            if let Err(e) = dekan_platform::shell::open_folder(dir) {
                                                warn!(error = %e, "Could not open the logs folder");
                                            }
                                        }
                                        None => warn!("Logs folder requested from the tray, but its path could not be resolved at boot"),
                                    }
                                }
                                dekan_platform::tray::TrayEvent::OpenTools => {
                                    let _ = std::fs::create_dir_all(&tray_tools_dir); // ignore-ok: create dir if missing before opening
                                    if let Err(e) = dekan_platform::shell::open_folder(&tray_tools_dir) {
                                        warn!(error = %e, tools = %tray_tools_dir.display(), "Could not open the tools folder");
                                    }
                                }
                                dekan_platform::tray::TrayEvent::About => {
                                    dekan_platform::welcome::show_about_window();
                                }
                                dekan_platform::tray::TrayEvent::ToggleAutostart => {
                                    match dekan_platform::autostart::toggle() {
                                        Ok(enabled) => info!(enabled, "Start with Windows changed from the tray"),
                                        Err(e) => warn!(error = %e, "Could not change the Start with Windows setting"),
                                    }
                                }
                                dekan_platform::tray::TrayEvent::ToggleAutoAccept => {
                                    match dekan_platform::auto_accept::toggle() {
                                        Ok(enabled) => info!(enabled, "Automatic match accept changed from the tray"),
                                        Err(e) => warn!(error = %e, "Could not change the automatic match accept setting"),
                                    }
                                }
                            }
                        }
                    }
                }
            }
            drop(tray);
        });

        let mut state_rx_tray = state_rx.clone();
        let tool_files = required_tools.clone();
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
                        let (party_line, in_room) = tray_party_line(&state.party_status, text);
                        tray_controller.update_party(&party_line, in_room);
                    }
                }
            }
        });
    }

    let auto_accept = dekan_platform::auto_accept::load();
    info!(
        enabled = auto_accept,
        "Automatic match accept setting loaded"
    );
    let state_rx_accept = state_rx.clone();
    supervisor.spawn("auto-accept", move |child_token| {
        dekan_app::auto_accept::run(state_rx_accept, child_token)
    });

    dekan_core::state::set_mod_selection(
        &state_tx,
        dekan_app::mods_store::load_selection(&state_dir_path),
    );
    let mods_config = dekan_app::overlay_session::ModsConfig {
        roots: paths.mod_roots.clone(),
        own_root: paths.custom_mods_root.clone(),
        state_dir: state_dir_path.clone(),
        game_dir: paths.game_dir.clone(),
        injection_tools: required_tools,
    };

    if instance_guard.is_some() {
        match dekan_platform::activation::ActivationListener::create(INSTANCE_NAME) {
            Ok(listener) => {
                supervisor.spawn("activation-listener", move |child_token| async move {
                    loop {
                        tokio::select! {
                            _ = child_token.cancelled() => break,
                            () = tokio::time::sleep(tokio::time::Duration::from_millis(400)) => {
                                if listener.take_request() {
                                    info!("Another launch was detected; surfacing this instance");
                                    dekan_platform::welcome::show_welcome_window();
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

    match dekan_platform::overlay_window::OverlayWindow::spawn() {
        Ok((overlay, commands)) => {
            let session = dekan_app::overlay_session::OverlaySession::new(
                overlay.controller(),
                commands,
                state_tx.clone(),
                state_rx.clone(),
                library_root,
                mods_config,
            );
            supervisor.spawn("overlay-session", move |child_token| async move {
                session.run(child_token).await;

                drop(overlay);
            });
        }
        Err(e) => {
            error!(error = %e, "Could not create the skin selection overlay");
        }
    }

    let injection_trigger =
        trigger::InjectionTrigger::new(state_tx.clone(), state_rx.clone(), paths);
    supervisor.spawn("injection-trigger", move |child_token| async move {
        injection_trigger.run(child_token).await;
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal (Ctrl+C)");
        }
        _ = shutdown_token.cancelled() => {
            info!("Shutdown token triggered");
        }
    }

    info!("Shutting down supervised tasks");
    supervisor.shutdown().await;
    let dropped = logging.dropped.dropped_lines();
    if dropped > 0 {
        warn!(
            dropped_lines = dropped,
            "Log lines were dropped this session because the writer queue was full"
        );
    }
    info!("Dekan shut down cleanly");
    drop(logging);

    Ok(())
}

fn tray_status(
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

fn tray_party_line(
    status: &dekan_core::party::PartyStatus,
    text: &'static dekan_platform::i18n::Text,
) -> (String, bool) {
    use dekan_core::party::PartyStatus;
    match status {
        PartyStatus::Off => (text.party_off.to_owned(), false),
        PartyStatus::Unavailable { .. } => (text.party_unavailable.to_owned(), false),
        PartyStatus::Connecting => (text.party_connecting.to_owned(), true),
        PartyStatus::Connected { members } => (
            dekan_platform::i18n::fill(text.party_in_room, "n", &members.to_string()),
            true,
        ),
        PartyStatus::Error { .. } => (text.party_reconnecting.to_owned(), true),
    }
}

/// Log whether the installed game build differs from the one seen on the last run.
///
/// Reads only the executable's PE headers (a few hundred bytes). An unknown game folder is not an
fn report_game_build(
    state_dir: &std::path::Path,
    game_dir: &std::path::Path,
    overlay_dir: &std::path::Path,
) {
    use dekan_platform::game_version::{self, BuildCheck};

    if game_dir.as_os_str().is_empty() {
        debug!("Game build not checked: the game folder is not known yet");
        return;
    }
    let stamp = match game_version::check(state_dir, game_dir) {
        Ok(BuildCheck::Unchanged { stamp }) => {
            debug!(
                time_date_stamp = stamp,
                "Game build unchanged since the last run"
            );
            stamp
        }
        Ok(BuildCheck::Changed { old, new }) => {
            info!(
                old,
                new, "Game build changed; invalidating overlay cache and locale (G2, G6)"
            );

            dekan_inject::overlay_cache::OverlayCache::invalidate(overlay_dir);
            dekan_app::catalog::invalidate_locale_cache();
            new
        }
        Ok(BuildCheck::FirstSeen { new }) => {
            info!(new, "Game build recorded for the first time");
            new
        }
        Err(e) => {
            warn!(
                game = %game_dir.join(game_version::GAME_EXE).display(),
                error = %e,
                "Could not read or record the game build"
            );
            return;
        }
    };
    report_ltk_dll_support(stamp);
}

fn report_ltk_dll_support(stamp: u32) {
    use dekan_inject::ltk_host::{DllSupport, LTK_DLL_GAME_BUILD_LIMIT, dll_support};

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    match dll_support(stamp, now) {
        DllSupport::Supported => debug!(
            game_build = stamp,
            dll_limit = LTK_DLL_GAME_BUILD_LIMIT,
            "The patcher DLL accepts the installed game build"
        ),
        DllSupport::SupportedUntilNextPatch { days_left } => warn!(
            game_build = stamp,
            dll_limit = LTK_DLL_GAME_BUILD_LIMIT,
            days_left,
            "The patcher DLL accepts this game build, but refuses builds made after its limit: the next game patch needs a refreshed DLL"
        ),
        DllSupport::Refused => error!(
            game_build = stamp,
            dll_limit = LTK_DLL_GAME_BUILD_LIMIT,
            "The installed game build is newer than the patcher DLL accepts; no skin can load until Dekan ships a refreshed DLL"
        ),
    }
}

fn remove_retired_bridge_files(state_dir: &std::path::Path) {
    for name in ["bridge.port", "bridge.token"] {
        let path = state_dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => info!(file = %path.display(), "Removed a file of the retired local bridge"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(
                file = %path.display(),
                error = %e,
                "Could not remove a file of the retired local bridge"
            ),
        }
    }
}

fn seed_directory(src: &std::path::Path, dst: &std::path::Path) {
    if !src.exists() {
        return;
    }

    // ignore-ok: best-effort creation of destination directory
    let _ = std::fs::create_dir_all(dst);
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let path = entry.path();
            let target = dst.join(entry.file_name());
            if path.is_dir() {
                seed_directory(&path, &target);
            } else if path.is_file()
                && !target.exists()
                && std::fs::hard_link(&path, &target).is_err()
            {
                // ignore-ok: fallback to file copy if hard link fails
                let _ = std::fs::copy(&path, &target);
            }
        }
    }
}
