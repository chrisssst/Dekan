#![windows_subsystem = "windows"]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use anyhow::Result;
use dekan_platform::paths::state_dir;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use dekan_app::catalog;

mod logging;
mod trigger;

const INSTANCE_NAME: &str = "dekan";

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(dekan_app::injector_install::INSTALL_FLAG) {
        std::process::exit(install_injector_elevated(&args[2..]));
    }

    let state_dir_path = state_dir().unwrap_or_else(|_| std::env::temp_dir().join("Dekan_state"));

    let mut lock_failure = None;
    let instance_guard = match dekan_platform::single_instance::SingleInstanceGuard::acquire(
        INSTANCE_NAME,
        Some(&state_dir_path),
    ) {
        Ok(guard) => Some(guard),
        Err(dekan_platform::error::PlatformError::AlreadyRunning { .. }) => {
            if !matches!(
                dekan_platform::activation::request_activation(INSTANCE_NAME),
                Ok(true)
            ) {
                dekan_platform::activation::notify_already_running();
            }
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

    info!(
        version = env!("CARGO_PKG_VERSION"),
        logs_dir = %logs_dir_path.display(),
        single_instance = instance_guard.is_some(),
        elevated,
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
    if let Some(e) = lock_failure {
        warn!(error = %e, "Single-instance lock unavailable; a second launch will not be blocked");
    }

    let mut paths = trigger::ResolvedPaths::discover();

    if injector_unusable(&paths, &state_dir_path).await {
        return Ok(());
    }

    let (state_tx, state_rx) = dekan_core::state::new_state_channel();

    let shutdown_token = CancellationToken::new();
    let mut supervisor = dekan_core::supervisor::Supervisor::new(shutdown_token.clone());

    remove_retired_files(&state_dir_path);

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
        dekan_inject::overlay_builder::persist_game_index_in(&state_dir_path);
        let gate_state = state_rx.clone();
        let gate_token = shutdown_token.clone();
        let game_dir = paths.game_dir.clone();
        let cache_dir = state_dir_path.clone();
        let spawned = std::thread::Builder::new()
            .name("dekan-prewarm".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                match dekan_inject::overlay_builder::get_or_index_game(&game_dir) {
                    Ok(index) => info!(
                        wads = index.len(),
                        elapsed_ms = started.elapsed().as_millis(),
                        "Game WAD index ready"
                    ),
                    Err(e) => warn!(error = %e, "Game WAD index not prewarmed; the first build indexes on demand"),
                }
                let _background = dekan_platform::process::BackgroundThread::enter();
                dekan_classic::generator::prewarm_companions(&game_dir, &cache_dir, || {
                    if gate_token.is_cancelled() {
                        dekan_classic::generator::PrewarmGate::Stop
                    } else if dekan_app::update_check::is_busy(gate_state.borrow().phase) {
                        dekan_classic::generator::PrewarmGate::Wait
                    } else {
                        dekan_classic::generator::PrewarmGate::Go
                    }
                });
            });
        if let Err(e) = spawned {
            warn!(error = %e, "Prewarm not started; the game is indexed and each champion scanned when needed");
        }
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

    let installed_dll = dekan_app::ltk_release::InstalledDllCache::default();
    let game_build = report_game_build(&state_dir_path, &paths.game_dir, &paths.overlay_dir);
    report_ltk_dll_support(game_build, installed_dll.read(&paths.ltk_dll_path));

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
    let random_skin = dekan_platform::preferences::RANDOM_SKIN.load();
    info!(
        enabled = random_skin,
        "Random skin when none is chosen setting loaded"
    );
    let light_loading = dekan_platform::preferences::LIGHT_LOADING.load();
    info!(
        enabled = light_loading,
        "Light match loading setting loaded"
    );

    let update_notice = dekan_app::update_check::UpdateNotice::default();
    let ltk_notice = dekan_app::ltk_release::LtkNotice::default();
    let live_game = dekan_app::live_game::LiveGame::default();
    let live_state = state_rx.clone();
    let live_game_dir = paths.game_dir.clone();
    let live_for_task = live_game.clone();
    let live_logs_dir = dekan_platform::paths::logs_dir().ok();
    supervisor.spawn("live-game", move |child_token| {
        dekan_app::live_game::run(
            live_state,
            live_game_dir,
            live_logs_dir,
            live_for_task,
            child_token,
        )
    });
    let mut panel_links: Option<dekan_platform::panel::PanelLinks> = None;
    if let Some(tray) = tray {
        let tray_shutdown = shutdown_token.clone();
        let tray_logs_dir = dekan_platform::paths::logs_dir().ok();
        let tray_tools_dir = paths.tools_dir.clone();
        let tray_mods_dir = paths.custom_mods_root.clone();
        let tray_controller = tray.controller();
        let mark_state = state_rx.clone();
        let mark_live = live_game.clone();
        if let Some(hotkey) =
            dekan_platform::hotkey::spawn_mark_problem_hotkey(tray_controller.events())
        {
            live_game.on_match(Box::new(move |playing| {
                if playing {
                    hotkey.arm();
                } else {
                    hotkey.disarm();
                }
            }));
        }
        let links = panel_links_for(
            &tray_controller,
            state_rx.clone(),
            required_tools.clone(),
            paths.game_dir.clone(),
            game_build,
            elevated,
            ReleaseNotices {
                dekan: update_notice.clone(),
                ltk: ltk_notice.clone(),
                installed: InstalledInjector {
                    dll: paths.ltk_dll_path.clone(),
                    cache: installed_dll.clone(),
                },
            },
        );
        panel_links = Some(links.clone());
        let (install_tx, mut install_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let install_tray = tray_controller.clone();
        let install_notice = ltk_notice.clone();
        let install_tools = paths.tools_dir.clone();
        let install_state = state_dir_path.clone();
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

        dekan_platform::welcome::show_welcome_window();

        let actions = TrayActions {
            shutdown: tray_shutdown,
            logs_dir: tray_logs_dir,
            tools_dir: tray_tools_dir,
            mods_dir: tray_mods_dir,
            party_tx: party_tx.clone(),
            links: links.clone(),
            mark_state,
            mark_live,
            install_tx,
        };

        supervisor.spawn("tray-events", move |child_token| async move {
            let mut tray = tray;
            loop {
                tokio::select! {
                    _ = child_token.cancelled() => break,
                    event = tray.recv_event() => match event {
                        Some(event) => actions.handle(event),
                        None => break,
                    },
                }
            }
            drop(tray);
        });

        if dekan_app::update_check::is_enabled(
            std::env::var(dekan_core::env::UPDATE_CHECK).ok().as_deref(),
        ) {
            let update_tray = tray_controller.clone();
            let check = dekan_app::update_check::UpdateCheck {
                state_dir: state_dir_path.clone(),
                notice: update_notice.clone(),
                notify: Box::new(move |latest| {
                    let text = dekan_platform::i18n::text();
                    update_tray.notify(
                        &dekan_platform::i18n::fill(
                            text.update_available_title,
                            "version",
                            &latest.to_string(),
                        ),
                        text.update_available_body,
                    );
                }),
            };
            let state_rx_update = state_rx.clone();
            supervisor.spawn("update-check", move |child_token| {
                dekan_app::update_check::run(check, state_rx_update, child_token)
            });

            let ltk_tray = tray_controller.clone();
            let ltk_check = dekan_app::ltk_release::LtkCheck {
                state_dir: state_dir_path.clone(),
                installed_dll: paths.ltk_dll_path.clone(),
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
        } else {
            info!("Update check turned off (DEKAN_UPDATE_CHECK)");
        }

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
                        let (party_line, in_room) = dekan_app::control_panel::party_line(&state.party_status, state.party_hosting, text);
                        tray_controller.update_party(&party_line, in_room);
                    }
                }
            }
        });
    }

    let auto_accept = dekan_platform::preferences::AUTO_ACCEPT.load();
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
        overlay_dir: paths.overlay_dir.clone(),
        injection_tools: required_tools,
    };

    if instance_guard.is_some() {
        match dekan_platform::activation::ActivationListener::create(INSTANCE_NAME) {
            Ok(listener) => {
                let activation_panel = panel_links.clone();
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

async fn compatible_ltk_version(state_dir: &std::path::Path) -> Option<String> {
    let online = dekan_app::update_check::is_enabled(
        std::env::var(dekan_core::env::UPDATE_CHECK).ok().as_deref(),
    );
    let cached = || {
        dekan_app::ltk_release::load_verdicts(state_dir)
            .status_from_cache()
            .and_then(|s| s.compatible)
    };
    if !online {
        return cached();
    }
    match tokio::time::timeout(
        STARTUP_LTK_LOOKUP,
        dekan_app::ltk_release::compatible_version(state_dir),
    )
    .await
    {
        Ok(found) => found,
        Err(_) => {
            debug!("LTK Manager lookup took too long at startup; using the last known result");
            cached()
        }
    }
}

const STARTUP_LTK_LOOKUP: std::time::Duration = std::time::Duration::from_secs(10);

fn install_injector_elevated(args: &[String]) -> i32 {
    let [staging, tools] = args else {
        return 2;
    };
    let tools = std::path::PathBuf::from(tools);
    let install_dir = dekan_platform::paths::install_dir().ok();
    let exe = std::env::current_exe().ok();
    if !dekan_app::injector_install::is_dekan_tools_folder(
        &tools,
        install_dir.as_deref(),
        exe.as_deref(),
    ) {
        return 3;
    }
    match dekan_app::injector_install::install(std::path::Path::new(staging), &tools) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

enum AutoInstall {
    Installed,
    Declined,
    Failed(String),
}

async fn install_injector_automatically(
    tools: &std::path::Path,
    state_dir: &std::path::Path,
    version: &str,
) -> AutoInstall {
    use dekan_app::injector_install::{InstallError, elevated_parameters, install, stage};
    use dekan_platform::elevation::{ElevatedRun, run_elevated};

    let staging = match stage(version, state_dir).await {
        Ok(staging) => staging,
        Err(e) => return AutoInstall::Failed(e.to_string()),
    };
    let outcome = match install(&staging, tools) {
        Ok(()) => AutoInstall::Installed,
        Err(InstallError::Denied) => {
            info!(tools = %tools.display(), "Asking Windows for permission to copy the injector into the tools folder");
            let parameters = elevated_parameters(&staging, tools);
            let elevated = match std::env::current_exe() {
                Ok(exe) => tokio::task::spawn_blocking(move || run_elevated(&exe, &parameters))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|run| run.map_err(|e| e.to_string())),
                Err(e) => Err(e.to_string()),
            };
            match elevated {
                Ok(ElevatedRun::Finished(0)) => AutoInstall::Installed,
                Ok(ElevatedRun::Finished(code)) => {
                    AutoInstall::Failed(format!("the elevated copy ended with code {code}"))
                }
                Ok(ElevatedRun::Declined) => AutoInstall::Declined,
                Err(e) => AutoInstall::Failed(e),
            }
        }
        Err(e) => AutoInstall::Failed(e.to_string()),
    };
    if let Err(e) = std::fs::remove_dir_all(&staging) {
        debug!(staging = %staging.display(), error = %e, "Injector staging folder not removed");
    }
    outcome
}

async fn injector_unusable(paths: &trigger::ResolvedPaths, state_dir: &std::path::Path) -> bool {
    use dekan_app::startup::{InjectorRefusal, injector_refusal};

    let text = dekan_platform::i18n::text();
    let (title, body) = match injector_refusal(&paths.ltk_host_exe, &paths.ltk_dll_path) {
        None => return false,
        Some(InjectorRefusal::Missing) => {
            warn!(
                tools = %paths.tools_dir.display(),
                "Dekan stopped at startup: the injector files are missing from the tools folder"
            );
            (text.missing_tools_title, text.missing_tools_body)
        }
        Some(InjectorRefusal::NotTrusted(files)) => {
            for (file, error) in &files {
                warn!(
                    file = %file.display(),
                    error = %error,
                    "Dekan stopped at startup: an injector file is not signed by its publisher"
                );
            }
            (text.broken_tools_title, text.broken_tools_body)
        }
    };
    let compatible = compatible_ltk_version(state_dir).await;
    info!(
        compatible = compatible.as_deref().unwrap_or("unknown"),
        "Pointing the user at the newest LTK Manager release with a signed injector"
    );
    let mut failure = None;
    if let Some(version) = compatible.as_deref() {
        let offer = dekan_platform::i18n::fill(text.injector_auto_body, "version", version);
        if dekan_platform::shell::message_box_question(text.injector_auto_title, &offer) {
            match install_injector_automatically(&paths.tools_dir, state_dir, version).await {
                AutoInstall::Installed => {
                    let still_refused = injector_refusal(&paths.ltk_host_exe, &paths.ltk_dll_path);
                    if still_refused.is_none() {
                        info!(version, tools = %paths.tools_dir.display(), "Injector installed from the LTK Manager release on GitHub");
                        return false;
                    }
                    warn!(refusal = ?still_refused, "The installed injector is still refused");
                }
                AutoInstall::Declined => {
                    info!("The user declined the permission to copy the injector")
                }
                AutoInstall::Failed(e) => {
                    warn!(error = %e, "The injector could not be installed automatically");
                    failure = Some(e);
                }
            }
        }
    }
    let version = compatible
        .clone()
        .unwrap_or_else(|| text.ltk_version_unknown.to_owned());
    let body = dekan_platform::i18n::fill(body, "version", &version);
    let body = match failure {
        Some(e) => format!(
            "{}\n\n{body}",
            dekan_platform::i18n::fill(text.injector_auto_failed, "error", &e)
        ),
        None => body,
    };
    let page = dekan_app::ltk_release::release_page(compatible.as_deref());
    if let Err(e) = dekan_platform::shell::open_web_page(&page) {
        warn!(error = %e, page = %page, "Could not open the LTK Manager release page");
    }
    if let Err(e) = std::fs::create_dir_all(&paths.tools_dir)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            dekan_platform::shell::open_folder(&paths.tools_dir).map_err(|e| e.to_string())
        })
    {
        warn!(tools = %paths.tools_dir.display(), error = %e, "The tools folder could not be opened for the user");
    }
    dekan_platform::shell::message_box_warning(title, &body);
    true
}

struct ReleaseNotices {
    dekan: dekan_app::update_check::UpdateNotice,
    ltk: dekan_app::ltk_release::LtkNotice,
    installed: InstalledInjector,
}

struct InstalledInjector {
    dll: std::path::PathBuf,
    cache: dekan_app::ltk_release::InstalledDllCache,
}

async fn install_from_panel(
    notice: &dekan_app::ltk_release::LtkNotice,
    tools: &std::path::Path,
    state_dir: &std::path::Path,
    tray: &dekan_platform::tray::TrayController,
) {
    let text = dekan_platform::i18n::text();
    let Some(version) = notice.status().and_then(|s| s.compatible) else {
        warn!(
            "Injector install requested, but no LTK Manager release with a signed injector is known yet"
        );
        return;
    };
    info!(version = %version, "Installing the injector requested from the control panel");
    match install_injector_automatically(tools, state_dir, &version).await {
        AutoInstall::Installed => {
            info!(version = %version, tools = %tools.display(), "Injector installed from the LTK Manager release on GitHub");
            tray.notify(
                &dekan_platform::i18n::fill(text.injector_installed_title, "version", &version),
                text.injector_installed_body,
            );
        }
        AutoInstall::Declined => {
            info!("The user declined the permission to copy the injector");
            tray.notify(text.injector_auto_title, text.injector_install_declined);
        }
        AutoInstall::Failed(e) => {
            warn!(error = %e, version = %version, "The injector could not be installed from the control panel");
            tray.notify(
                text.injector_auto_title,
                &dekan_platform::i18n::fill(text.injector_auto_failed, "error", &e),
            );
        }
    }
}

fn panel_links_for(
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
        };
        dekan_app::control_panel::snapshot(&facts, dekan_platform::i18n::text())
    };
    dekan_platform::panel::PanelLinks {
        events: tray.events(),
        snapshot: std::sync::Arc::new(snapshot),
    }
}

fn report_game_build(
    state_dir: &std::path::Path,
    game_dir: &std::path::Path,
    overlay_dir: &std::path::Path,
) -> Option<u32> {
    use dekan_platform::game_version::{self, BuildCheck};

    if game_dir.as_os_str().is_empty() {
        debug!("Game build not checked: the game folder is not known yet");
        return None;
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
                game = %game_version::game_exe(game_dir).display(),
                error = %e,
                "Could not read or record the game build"
            );
            return None;
        }
    };
    Some(stamp)
}

fn report_ltk_dll_support(
    stamp: Option<u32>,
    installed: Option<dekan_app::ltk_release::InstalledDll>,
) {
    use dekan_inject::ltk_host::{DllSupport, dll_support};

    let Some(stamp) = stamp else { return };
    let Some(limit) = installed.and_then(|dll| dll.build_limit) else {
        warn!(
            game_build = stamp,
            "The patcher DLL's game build limit could not be read; Dekan cannot tell whether it accepts this build"
        );
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    match dll_support(stamp, limit, now) {
        DllSupport::Supported => debug!(
            game_build = stamp,
            dll_limit = limit,
            "The patcher DLL accepts the installed game build"
        ),
        DllSupport::SupportedUntilNextPatch { days_left } => warn!(
            game_build = stamp,
            dll_limit = limit,
            days_left,
            "The patcher DLL accepts this game build, but refuses builds made after its limit: the next game patch needs a refreshed DLL"
        ),
        DllSupport::Refused => error!(
            game_build = stamp,
            dll_limit = limit,
            "The installed game build is newer than the patcher DLL accepts; no skin can load until a refreshed DLL is installed"
        ),
    }
}

fn remove_retired_files(state_dir: &std::path::Path) {
    for name in ["bridge.port", "bridge.token", "suspend.lock"] {
        let path = state_dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => info!(file = %path.display(), "Removed a file of a retired feature"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(
                file = %path.display(),
                error = %e,
                "Could not remove a file of a retired feature"
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

struct TrayActions {
    shutdown: CancellationToken,
    logs_dir: Option<std::path::PathBuf>,
    tools_dir: std::path::PathBuf,
    mods_dir: std::path::PathBuf,
    party_tx: tokio::sync::mpsc::UnboundedSender<dekan_app::party_manager::PartyCommand>,
    links: dekan_platform::panel::PanelLinks,
    mark_state: dekan_core::state::StateReceiver,
    mark_live: dekan_app::live_game::LiveGame,
    install_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

impl TrayActions {
    fn handle(&self, event: dekan_platform::tray::TrayEvent) {
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
