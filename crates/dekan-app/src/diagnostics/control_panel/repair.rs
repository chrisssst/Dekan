use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use dekan_platform::i18n::{Text, fill};
use dekan_platform::panel::PanelRepair;

pub const REPAIR_STATE_IDLE: u8 = 0;
pub const REPAIR_STATE_BUSY: u8 = 1;
pub const REPAIR_STATE_OK: u8 = 2;
pub const REPAIR_STATE_ERROR: u8 = 3;

#[derive(Debug, Clone, Default)]
pub struct RepairState(Arc<Mutex<PanelRepair>>);

impl RepairState {
    #[must_use]
    pub fn snapshot(&self) -> PanelRepair {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn begin_skins(&self, message: &str) -> bool {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.skins_busy {
            return false;
        }
        guard.skins_busy = true;
        guard.state = REPAIR_STATE_BUSY;
        guard.message = message.to_owned();
        true
    }

    pub fn finish_skins(&self, ok: bool, message: String) {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.skins_busy = false;
        if !guard.client_busy {
            guard.state = if ok {
                REPAIR_STATE_OK
            } else {
                REPAIR_STATE_ERROR
            };
            guard.message = message;
        }
    }

    pub fn begin_client(&self, message: &str) -> bool {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.client_busy {
            return false;
        }
        guard.client_busy = true;
        guard.state = REPAIR_STATE_BUSY;
        guard.message = message.to_owned();
        true
    }

    pub fn finish_client(&self, ok: bool, message: String) {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.client_busy = false;
        if !guard.skins_busy {
            guard.state = if ok {
                REPAIR_STATE_OK
            } else {
                REPAIR_STATE_ERROR
            };
            guard.message = message;
        }
    }
}

#[derive(Debug, Default)]
pub struct RepairSkinsReport {
    pub cleared_generated: usize,
    pub checked: usize,
    pub repaired: usize,
    pub incompatible: usize,
    pub game_found: bool,
    pub totals: Option<crate::mod_repair::ScanSummary>,
}

#[derive(Debug, Default)]
pub struct RepairClientReport {
    pub cleared_caches: usize,
    pub stopped_processes: usize,
    pub reset_bridge: bool,
    pub warnings: Vec<String>,
}

pub fn repair_skins_and_mods(
    mod_roots: &[PathBuf],
    state_dir: &Path,
    game_dir: &Path,
) -> RepairSkinsReport {
    let mut cleared_generated = 0usize;
    let repair_index = state_dir.join("mod_repair.json");
    if repair_index.exists() && std::fs::remove_file(&repair_index).is_ok() {
        cleared_generated += 1;
    }
    if let Ok(data_dir) = dekan_platform::paths::data_dir() {
        cleared_generated += clear_generated_skin_dirs(&data_dir.join("mods"));
    }
    let resolved_game = dekan_platform::paths::normalize_game_dir(game_dir)
        .or_else(dekan_platform::paths::discover_game_dir);
    let Some(resolved_game) = resolved_game else {
        return RepairSkinsReport {
            cleared_generated,
            game_found: false,
            ..RepairSkinsReport::default()
        };
    };
    let report =
        crate::mod_repair::repair_custom_mods(mod_roots, &resolved_game, state_dir, &|| false);
    RepairSkinsReport {
        cleared_generated,
        checked: report.checked,
        repaired: report.repaired.len(),
        incompatible: report.incompatible.len(),
        game_found: true,
        totals: report.totals,
    }
}

pub(crate) fn clear_generated_skin_dirs(generated_dir: &Path) -> usize {
    let mut cleared = 0usize;
    if let Ok(entries) = std::fs::read_dir(generated_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(s) = name.to_str()
                && (s.starts_with("std_") || s == "dekan_loading_name")
                && std::fs::remove_dir_all(entry.path()).is_ok()
            {
                cleared += 1;
            }
        }
    }
    cleared
}

fn remove_path_tracked(path: &Path, report: &mut RepairClientReport) {
    if !path.exists() {
        return;
    }
    let res = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match res {
        Ok(()) => report.cleared_caches += 1,
        Err(e) => report.warnings.push(format!("{}: {e}", path.display())),
    }
}

pub fn repair_client(state_dir: &Path) -> RepairClientReport {
    let mut report = RepairClientReport::default();

    #[cfg(windows)]
    {
        for _ in 0..8 {
            let Some(pid) = dekan_platform::process::ProcessFinder::find_any_process(&[
                "ltk-manager.exe",
                "LTK Manager.exe",
                "ltk_manager.exe",
                "LeagueToolkit Manager.exe",
                "dekan_launcher.exe",
            ])
            .ok()
            .flatten() else {
                break;
            };
            if dekan_platform::process::terminate_process(pid) {
                report.stopped_processes += 1;
            } else {
                break;
            }
        }
        if report.stopped_processes > 0 {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    if let Ok(data_dir) = dekan_platform::paths::data_dir() {
        for sub in ["staging", "overlay"] {
            remove_path_tracked(&data_dir.join(sub), &mut report);
        }
    }
    remove_path_tracked(&state_dir.join("ltf_journal.json"), &mut report);

    let mut local_roots = Vec::new();
    if let Ok(desktop_local) = dekan_platform::user_profile::local_app_data() {
        local_roots.push(desktop_local);
    }
    if let Ok(env_local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(env_local);
        if !local_roots.contains(&p) {
            local_roots.push(p);
        }
    }
    for local_root in &local_roots {
        let bridge_dir = local_root.join("DekanBridge");
        for file in ["port.txt", "token.txt"] {
            let path = bridge_dir.join(file);
            if path.exists() {
                remove_path_tracked(&path, &mut report);
                report.reset_bridge = true;
            }
        }
    }

    let mut roaming_roots = Vec::new();
    for local_root in &local_roots {
        if let Some(roaming) = local_root.parent().map(|p| p.join("Roaming"))
            && !roaming_roots.contains(&roaming)
        {
            roaming_roots.push(roaming);
        }
    }
    if let Ok(roaming) = std::env::var("APPDATA") {
        let p = PathBuf::from(roaming);
        if !roaming_roots.contains(&p) {
            roaming_roots.push(p);
        }
    }
    for roaming in roaming_roots {
        let profile = roaming
            .join("dev.leaguetoolkit.manager")
            .join("profiles")
            .join("default");
        for item in ["overlay", "overlay.json", "override_meta.bin"] {
            remove_path_tracked(&profile.join(item), &mut report);
        }
    }

    report
}

#[must_use]
pub fn format_skins_feedback(report: &RepairSkinsReport, text: &Text) -> (bool, String) {
    let cleared = report.cleared_generated.to_string();
    if !report.game_found {
        return (
            false,
            fill(text.panel_repair_skins_no_game, "cleared", &cleared),
        );
    }
    let checked = report.checked.to_string();
    let repaired = report.repaired.to_string();
    let incompatible = report.incompatible.to_string();
    if report.incompatible > 0 {
        let msg = fill(
            &fill(
                &fill(text.panel_repair_skins_done_warn, "cleared", &cleared),
                "repaired",
                &repaired,
            ),
            "incompatible",
            &incompatible,
        );
        return (false, msg);
    }
    let msg = fill(
        &fill(
            &fill(text.panel_repair_skins_done, "cleared", &cleared),
            "checked",
            &checked,
        ),
        "repaired",
        &repaired,
    );
    (true, msg)
}

#[must_use]
pub fn format_client_feedback(report: &RepairClientReport, text: &Text) -> (bool, String) {
    let caches = report.cleared_caches.to_string();
    if let Some(first) = report.warnings.first() {
        let msg = fill(
            &fill(text.panel_repair_client_partial, "caches", &caches),
            "error",
            first,
        );
        return (false, msg);
    }
    (true, fill(text.panel_repair_client_done, "caches", &caches))
}
