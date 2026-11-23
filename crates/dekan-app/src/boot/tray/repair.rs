use std::path::PathBuf;

use tracing::info;

pub fn spawn_repair_skins(
    repair_state: dekan_app::control_panel::RepairState,
    custom_mods: dekan_app::mod_repair::CustomModsNotice,
    mod_roots: Vec<PathBuf>,
    state_dir: PathBuf,
    notifier: dekan_platform::tray::TrayController,
) {
    let text = dekan_platform::i18n::text();
    if !repair_state.begin_skins(text.panel_repair_skins_running) {
        return;
    }
    dekan_platform::panel::refresh_open_panel();
    let game = dekan_platform::paths::discover_game_dir().unwrap_or_default();
    drop(tokio::task::spawn_blocking(move || {
        let report = dekan_app::control_panel::repair_skins_and_mods(&mod_roots, &state_dir, &game);
        if let Some(totals) = report.totals {
            custom_mods.set(totals);
        }
        info!(
            cleared = report.cleared_generated,
            checked = report.checked,
            repaired = report.repaired,
            incompatible = report.incompatible,
            game_found = report.game_found,
            "Manual skin and mod repair completed"
        );
        let t = dekan_platform::i18n::text();
        let (ok, message) = dekan_app::control_panel::format_skins_feedback(&report, t);
        repair_state.finish_skins(ok, message.clone());
        dekan_platform::panel::refresh_open_panel();
        notifier.notify(t.panel_repair_skins, &message);
    }));
}

pub fn spawn_repair_client(
    repair_state: dekan_app::control_panel::RepairState,
    state_dir: PathBuf,
    notifier: dekan_platform::tray::TrayController,
) {
    let text = dekan_platform::i18n::text();
    if !repair_state.begin_client(text.panel_repair_client_running) {
        return;
    }
    dekan_platform::panel::refresh_open_panel();
    drop(tokio::task::spawn_blocking(move || {
        let report = dekan_app::control_panel::repair_client(&state_dir);
        info!(
            cleared_caches = report.cleared_caches,
            stopped_processes = report.stopped_processes,
            reset_bridge = report.reset_bridge,
            warnings = report.warnings.len(),
            "Manual client and Dekan cache repair completed"
        );
        let t = dekan_platform::i18n::text();
        let (ok, message) = dekan_app::control_panel::format_client_feedback(&report, t);
        repair_state.finish_client(ok, message.clone());
        dekan_platform::panel::refresh_open_panel();
        notifier.notify(t.panel_repair_client, &message);
    }));
}
