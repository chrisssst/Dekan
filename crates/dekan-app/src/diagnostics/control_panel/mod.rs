use std::path::PathBuf;

use dekan_core::party::PartyStatus;
use dekan_inject::ltk_host::{DllSupport, dll_support};
use dekan_platform::i18n::{Text, fill};
use dekan_platform::panel::{PanelCheck, PanelSnapshot};

use crate::ltk_release::{InstalledDll, LtkStatus};

mod export;
mod redact;

pub use export::export_diagnostics;

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
    pub game_found: bool,
    pub lcu_connected: bool,
    pub game_build: Option<u32>,
    pub now_secs: u64,
    pub elevated: bool,
    pub update: Option<String>,
    pub ltk: Option<LtkStatus>,
    pub installed_dll: Option<InstalledDll>,
    pub custom_mods: Option<crate::mod_repair::ScanSummary>,
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
    let limit = facts.installed_dll.as_ref().and_then(|dll| dll.build_limit);
    let dll = match facts
        .game_build
        .zip(limit)
        .map(|(stamp, limit)| dll_support(stamp, limit, facts.now_secs))
    {
        Some(DllSupport::Supported) => check(text.check_dll, true, text.detail_ok.to_owned()),
        Some(DllSupport::SupportedUntilNextPatch { days_left }) if days_left < 0 => {
            check(text.check_dll, false, text.detail_dll_past_limit.to_owned())
        }
        Some(DllSupport::SupportedUntilNextPatch { days_left }) => check(
            text.check_dll,
            false,
            fill(text.detail_dll_days_left, "n", &days_left.to_string()),
        ),
        Some(DllSupport::Refused) => {
            check(text.check_dll, false, text.detail_dll_refused.to_owned())
        }
        None => check(text.check_dll, false, text.detail_dll_unknown.to_owned()),
    };
    let installed_sha = facts.installed_dll.as_ref().map(|dll| dll.sha256.as_str());
    let offered = facts
        .ltk
        .as_ref()
        .and_then(|status| status.offers_update_over(installed_sha));
    let untrusted_latest = facts
        .ltk
        .as_ref()
        .filter(|status| !status.latest_trusted)
        .map(|status| status.latest.as_str());
    let ltk_check = match (&facts.ltk, offered, untrusted_latest) {
        (None, _, _) => check(text.check_ltk, true, text.detail_ltk_unchecked.to_owned()),
        (Some(_), Some(version), _) if facts.tools_present => check(
            text.check_ltk,
            false,
            fill(text.detail_ltk_new, "version", version),
        ),
        (Some(_), _, Some(latest)) => check(
            text.check_ltk,
            false,
            fill(text.detail_ltk_untrusted, "latest", latest),
        ),
        (Some(status), _, None) => match &status.compatible {
            Some(version) => check(
                text.check_ltk,
                true,
                fill(text.detail_ltk_current, "version", version),
            ),
            None => check(text.check_ltk, true, text.detail_ltk_unchecked.to_owned()),
        },
    };
    let compatible = facts.ltk.as_ref().and_then(|s| s.compatible.as_deref());
    let (ltk_line, ltk_download) = if !facts.tools_present {
        let version = compatible.unwrap_or(text.ltk_version_unknown);
        (
            Some(fill(text.panel_ltk_missing_line, "version", version)),
            compatible.map(|version| fill(text.panel_ltk_download, "version", version)),
        )
    } else if let Some(version) = offered {
        (
            Some(fill(text.panel_ltk_new_line, "version", version)),
            Some(fill(text.panel_ltk_download, "version", version)),
        )
    } else {
        (
            untrusted_latest.map(|latest| fill(text.panel_ltk_untrusted_line, "latest", latest)),
            None,
        )
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
        ltk_line,
        ltk_download,
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
            dll,
            ltk_check,
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
