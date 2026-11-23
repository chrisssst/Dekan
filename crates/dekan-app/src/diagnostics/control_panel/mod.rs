use std::path::PathBuf;

use dekan_core::party::PartyStatus;
use dekan_platform::i18n::{Text, fill};
use dekan_platform::panel::{PanelCheck, PanelSnapshot};

mod export;
mod redact;
mod repair;

pub use export::export_diagnostics;
pub use repair::{
    REPAIR_STATE_BUSY, REPAIR_STATE_ERROR, REPAIR_STATE_IDLE, REPAIR_STATE_OK, RepairClientReport,
    RepairSkinsReport, RepairState, format_client_feedback, format_skins_feedback, repair_client,
    repair_skins_and_mods,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerStatus {
    Ready { version: Option<String> },
    Outdated { found: String, required: String },
    Missing,
}

#[derive(Debug, Clone)]
pub struct Facts {
    pub status: String,
    pub party_line: String,
    pub in_room: bool,
    pub auto_accept: bool,
    pub injection: bool,
    pub random_skin: bool,
    pub light_loading: bool,
    pub autostart: bool,
    pub tools_present: bool,
    pub manager: ManagerStatus,
    pub game_found: bool,
    pub lcu_connected: bool,
    pub elevated: bool,
    pub update: Option<String>,
    pub custom_mods: Option<crate::mod_repair::ScanSummary>,
    pub stats: dekan_platform::panel::PanelStats,
    pub repair: dekan_platform::panel::PanelRepair,
}

#[must_use]
pub fn party_line(status: &PartyStatus, hosting: bool, text: &Text) -> (String, bool) {
    match status {
        PartyStatus::Off => (text.party_off.to_owned(), false),
        PartyStatus::Unavailable { .. } => (text.party_unavailable.to_owned(), false),
        PartyStatus::Connecting if hosting => (text.party_created_connecting.to_owned(), true),
        PartyStatus::Connecting => (text.party_connecting.to_owned(), true),
        PartyStatus::Connected { members } => {
            let line = if hosting {
                text.party_created_in_room
            } else {
                text.party_in_room
            };
            (fill(line, "n", &members.to_string()), true)
        }
        PartyStatus::Error { .. } => (text.party_reconnecting.to_owned(), true),
    }
}

#[must_use]
pub fn snapshot(facts: &Facts, text: &Text) -> PanelSnapshot {
    let check = |label: &str, ok: bool, detail: String| PanelCheck {
        label: label.to_owned(),
        ok,
        detail,
    };
    let custom_mods = facts.custom_mods.map(|summary| {
        check(
            text.check_custom_mods,
            summary.refused == 0,
            fill(
                &fill(
                    text.detail_custom_mods,
                    "repaired",
                    &summary.repaired.to_string(),
                ),
                "refused",
                &summary.refused.to_string(),
            ),
        )
    });
    PanelSnapshot {
        status: facts.status.clone(),
        party_line: facts.party_line.clone(),
        in_room: facts.in_room,
        auto_accept: facts.auto_accept,
        injection: facts.injection,
        random_skin: facts.random_skin,
        light_loading: facts.light_loading,
        autostart: facts.autostart,
        update_line: facts.update.as_deref().map(|latest| {
            fill(
                &fill(text.panel_update_line, "version", latest),
                "current",
                dekan_platform::version::display_version(),
            )
        }),
        stats: facts.stats.clone(),
        repair: facts.repair.clone(),
        checks: vec![
            check(
                text.check_injector,
                facts.tools_present,
                if facts.tools_present {
                    text.detail_ok
                } else {
                    text.detail_injector_missing
                }
                .to_owned(),
            ),
            {
                let (ok, detail) = match &facts.manager {
                    ManagerStatus::Ready { version: Some(v) } => {
                        (true, format!("{} (v{v})", text.detail_ok))
                    }
                    ManagerStatus::Ready { version: None } => (true, text.detail_ok.to_owned()),
                    ManagerStatus::Outdated { found, required } => (
                        false,
                        fill(
                            &fill(text.detail_manager_outdated, "found", found),
                            "required",
                            required,
                        ),
                    ),
                    ManagerStatus::Missing => (false, text.detail_manager_missing.to_owned()),
                };
                check(text.check_manager, ok, detail)
            },
            check(
                text.check_game,
                facts.game_found,
                if facts.game_found {
                    text.detail_ok
                } else {
                    text.detail_game_missing
                }
                .to_owned(),
            ),
            check(
                text.check_client,
                facts.lcu_connected,
                if facts.lcu_connected {
                    text.detail_client_connected
                } else {
                    text.detail_client_waiting
                }
                .to_owned(),
            ),
            check(
                text.check_privileges,
                true,
                if facts.elevated {
                    text.detail_elevated
                } else {
                    text.detail_not_elevated
                }
                .to_owned(),
            ),
        ]
        .into_iter()
        .chain(custom_mods)
        .collect(),
    }
}

#[must_use]
pub fn game_found(configured: &std::path::Path) -> bool {
    dekan_platform::paths::is_valid_game_dir(configured)
        || dekan_platform::paths::discover_game_dir().is_some()
}

#[must_use]
pub fn tools_present(files: &[PathBuf]) -> bool {
    files.iter().all(|file| file.is_file())
}

#[cfg(test)]
mod tests;
