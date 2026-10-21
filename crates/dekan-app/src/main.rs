#![windows_subsystem = "windows"]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use anyhow::Result;
use dekan_platform::paths::state_dir;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

mod boot;
mod logging;
mod trigger;

const INSTANCE_NAME: &str = "dekan";

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(dekan_app::injector_install::INSTALL_FLAG) {
        std::process::exit(boot::injector::install_injector_elevated(&args[2..]));
    }

    let state_dir_resolved = state_dir();
    let state_dir_path = state_dir_resolved
        .as_ref()
        .map_or_else(|_| std::env::temp_dir().join("Dekan_state"), Clone::clone);

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

    let logs_dir_resolved = dekan_platform::paths::logs_dir();
    let logs_dir_path = logs_dir_resolved.as_ref().map_or_else(
        |_| std::env::temp_dir().join("Dekan").join("logs"),
        Clone::clone,
    );
    let logging = logging::init_or_explain(&logs_dir_path)?;
    logging::install_panic_hook();
    let elevated = dekan_platform::elevation::is_elevated();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        logs_dir = %logs_dir_path.display(),
        single_instance = instance_guard.is_some(),
        elevated,
        "Dekan starting"
    );

    boot::reports::log_profile_resolution();
    boot::reports::report_dir_fallback("state", &state_dir_resolved, &state_dir_path);
    boot::reports::report_dir_fallback("logs", &logs_dir_resolved, &logs_dir_path);
    if let Some(e) = lock_failure {
        warn!(error = %e, "Single-instance lock unavailable; a second launch will not be blocked");
    }

    let mut paths = trigger::ResolvedPaths::discover();

    if boot::injector::injector_unusable(&paths, &state_dir_path).await {
        return Ok(());
    }

    let (state_tx, state_rx) = dekan_core::state::new_state_channel();

    let shutdown_token = CancellationToken::new();
    let mut supervisor = dekan_core::supervisor::Supervisor::new(shutdown_token.clone());

    let discord_state = state_rx.clone();

    supervisor.spawn("discord-presence", move |child_token| {
        dekan_app::discord_presence::run(discord_state, child_token)
    });

    let pending_update = dekan_app::self_update::PendingUpdate::default();

    boot::reports::remove_retired_files(&state_dir_path);

    let lcu_observer = dekan_lcu::observer::LcuObserver::new(state_tx.clone(), None);
    supervisor.spawn_essential("lcu-observer", move |child_token| async move {
        lcu_observer.run(child_token).await;
    });

    boot::tasks::spawn_log_drop_monitor(&mut supervisor, logging.dropped.clone());

    let (party_tx, party_rx) =
        tokio::sync::mpsc::unbounded_channel::<dekan_app::party_manager::PartyCommand>();
    {
        let manager = dekan_app::party_manager::PartyManager::new(
            state_tx.clone(),
            state_rx.clone(),
            state_dir_path.clone(),
        );
        supervisor.spawn_essential("party-manager", move |child_token| async move {
            manager.run(child_token, party_rx).await;
        });
    }

    let (library_root, library_has_content) = boot::library::resolve_library(&mut paths);

    let custom_mods_notice = dekan_app::mod_repair::CustomModsNotice::default();
    let (custom_mods_tx, custom_mods_rx) =
        std::sync::mpsc::channel::<dekan_app::mod_repair::ScanSummary>();
    boot::library::spawn_prewarm(
        &paths,
        &state_dir_path,
        &state_rx,
        &shutdown_token,
        &custom_mods_notice,
        custom_mods_tx,
    );

    if library_has_content {
        info!(library = %library_root.display(), "Skin library resolved");
    } else {
        warn!(
            library = %library_root.display(),
            "Skin library is empty or missing; official skins will be generated from the installed game"
        );
    }

    boot::library::spawn_skin_sync(&mut supervisor, &library_root);

    let installed_dll = dekan_app::ltk_release::InstalledDllCache::default();
    let game_build =
        boot::reports::report_game_build(&state_dir_path, &paths.game_dir, &paths.overlay_dir);
    boot::reports::report_ltk_dll_support(game_build, installed_dll.read(&paths.ltk_dll_path));
    boot::reports::report_injector_files(&[&paths.ltk_host_exe, &paths.ltk_dll_path]);

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
    let injection = dekan_platform::preferences::INJECTION.load();
    info!(enabled = injection, "Skin injection setting loaded");
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
    let live_logs_dir = Some(logs_dir_path.clone());
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
        let tray_logs_dir = Some(logs_dir_path.clone());
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
        let links = boot::panel::panel_links_for(
            &tray_controller,
            state_rx.clone(),
            required_tools.clone(),
            paths.game_dir.clone(),
            game_build,
            elevated,
            boot::panel::ReleaseNotices {
                dekan: update_notice.clone(),
                ltk: ltk_notice.clone(),
                installed: boot::panel::InstalledInjector {
                    dll: paths.ltk_dll_path.clone(),
                    cache: installed_dll.clone(),
                },
                custom_mods: custom_mods_notice.clone(),
            },
        );
        boot::tasks::spawn_custom_mods_notice(
            &mut supervisor,
            tray_controller.clone(),
            custom_mods_rx,
        );
        panel_links = Some(links.clone());
        let install_tx = boot::tasks::spawn_injector_install(
            &mut supervisor,
            &ltk_notice,
            &paths.tools_dir,
            &state_dir_path,
            &tray_controller,
        );

        dekan_platform::welcome::show_welcome_window();

        let actions = boot::panel::TrayActions {
            shutdown: tray_shutdown,
            logs_dir: tray_logs_dir,
            tools_dir: tray_tools_dir,
            mods_dir: tray_mods_dir,
            mod_roots: paths
                .mod_roots
                .iter()
                .map(|root| root.path.clone())
                .collect(),
            state_dir: state_dir_path.clone(),
            notifier: tray_controller.clone(),
            party_tx: party_tx.clone(),
            links: links.clone(),
            mark_state,
            mark_live,
            install_tx,
        };

        supervisor.spawn_essential("tray-events", move |child_token| async move {
            let mut tray = tray;
            loop {
                tokio::select! {
                    _ = child_token.cancelled() => break,
                    event = tray.recv_event() => match event {
                        Some(event) => actions.handle(event),
                        None => {
                            error!("The tray event channel closed; the tray icon no longer responds");
                            break;
                        }
                    },
                }
            }
            drop(tray);
        });

        if dekan_app::update_check::is_enabled(
            std::env::var(dekan_core::env::UPDATE_CHECK).ok().as_deref(),
        ) {
            boot::tasks::spawn_update_check(
                &mut supervisor,
                boot::update::UpdateOffer {
                    tray: tray_controller.clone(),
                    state_dir: state_dir_path.clone(),
                    state_rx: state_rx.clone(),
                    pending: pending_update.clone(),
                    shutdown: shutdown_token.clone(),
                },
                &update_notice,
            );
            boot::tasks::spawn_ltk_check(
                &mut supervisor,
                &tray_controller,
                &state_dir_path,
                &paths.ltk_dll_path,
                &installed_dll,
                &ltk_notice,
                &state_rx,
            );
        } else {
            info!("Update check turned off (DEKAN_UPDATE_CHECK)");
        }

        boot::tasks::spawn_tray_status(
            &mut supervisor,
            &state_rx,
            tray_controller,
            required_tools.clone(),
        );
    }

    let auto_accept = dekan_platform::preferences::AUTO_ACCEPT.load();
    info!(
        enabled = auto_accept,
        "Automatic match accept setting loaded"
    );
    let state_rx_accept = state_rx.clone();
    supervisor.spawn_essential("auto-accept", move |child_token| {
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
        boot::tasks::spawn_activation_listener(&mut supervisor, panel_links);
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
            supervisor.spawn_essential("overlay-session", move |child_token| async move {
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
    supervisor.spawn_essential("injection-trigger", move |child_token| async move {
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
    let update = boot::update::prepare_install(&pending_update);
    boot::update::run_installer(update, instance_guard);
    drop(logging);

    Ok(())
}
