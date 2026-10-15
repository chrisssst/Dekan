use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Turkish,
    English,
}

impl Language {
    #[must_use]
    pub fn from_locale(locale: &str) -> Option<Self> {
        let language = locale.split(['_', '-']).next()?.to_ascii_lowercase();
        match language.as_str() {
            "tr" => Some(Self::Turkish),
            "en" => Some(Self::English),
            _ => None,
        }
    }

    #[must_use]
    pub fn of_windows() -> Self {
        static CACHED: OnceLock<Language> = OnceLock::new();
        *CACHED.get_or_init(|| {
            let lang_id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
            Self::from_primary_lang_id(lang_id & 0x3ff)
        })
    }

    fn from_primary_lang_id(primary: u16) -> Self {
        match primary {
            0x09 => Self::English,
            _ => Self::Turkish,
        }
    }

    #[must_use]
    pub fn for_locale(locale: Option<&str>) -> Self {
        locale.and_then(Self::from_locale).unwrap_or(Self::Turkish)
    }

    #[must_use]
    pub fn text(self) -> &'static Text {
        match self {
            Self::Turkish => &TURKISH,
            Self::English => &ENGLISH,
        }
    }
}

static ACTIVE_LANGUAGE: std::sync::RwLock<Option<Language>> = std::sync::RwLock::new(None);

pub fn set_active_locale(locale: &str) {
    if let Some(lang) = Language::from_locale(locale) {
        if let Ok(mut lock) = ACTIVE_LANGUAGE.write() {
            *lock = Some(lang);
        }
    }
}

pub fn reset_active_language() {
    if let Ok(mut lock) = ACTIVE_LANGUAGE.write() {
        *lock = None;
    }
}

#[must_use]
pub fn active_language() -> Language {
    if let Ok(lock) = ACTIVE_LANGUAGE.read() {
        if let Some(lang) = *lock {
            return lang;
        }
    }
    Language::Turkish
}

#[must_use]
pub fn text() -> &'static Text {
    active_language().text()
}

#[derive(Debug)]
pub struct Text {
    pub status_tools_missing: &'static str,
    pub status_waiting_league: &'static str,
    pub status_connected: &'static str,
    pub status_lobby: &'static str,
    pub status_matchmaking: &'static str,
    pub status_ready_check: &'static str,
    pub status_champ_select: &'static str,
    pub status_finalization: &'static str,
    pub status_injecting: &'static str,
    pub status_in_game: &'static str,
    pub status_in_game_confirmed: &'static str,
    pub status_in_game_unconfirmed: &'static str,
    pub status_in_game_failed: &'static str,
    pub status_reconnecting: &'static str,

    pub party_off: &'static str,
    pub party_unavailable: &'static str,
    pub party_connecting: &'static str,
    pub party_in_room: &'static str,
    pub party_reconnecting: &'static str,
    pub party_created_connecting: &'static str,
    pub party_created_in_room: &'static str,

    pub menu_party_create: &'static str,
    pub menu_party_join: &'static str,
    pub menu_party_leave: &'static str,
    pub menu_group_party: &'static str,
    pub menu_group_folders: &'static str,
    pub folder_mods: &'static str,
    pub folder_logs: &'static str,
    pub folder_tools: &'static str,
    pub menu_about: &'static str,
    pub menu_autostart: &'static str,
    pub menu_auto_accept: &'static str,
    pub menu_quit: &'static str,
    pub menu_open_panel: &'static str,
    pub menu_random_skin: &'static str,
    pub panel_section_options: &'static str,
    pub panel_theme: &'static str,
    pub panel_theme_classic: &'static str,
    pub panel_theme_crimson: &'static str,
    pub panel_theme_obsidian: &'static str,
    pub panel_section_diagnostics: &'static str,
    pub panel_random_skin_hint: &'static str,
    pub menu_light_loading: &'static str,
    pub panel_light_loading_hint: &'static str,
    pub check_injector: &'static str,
    pub check_game: &'static str,
    pub check_client: &'static str,
    pub check_dll: &'static str,
    pub check_privileges: &'static str,
    pub detail_ok: &'static str,
    pub detail_injector_missing: &'static str,
    pub detail_game_missing: &'static str,
    pub detail_client_connected: &'static str,
    pub detail_client_waiting: &'static str,
    pub detail_dll_days_left: &'static str,
    pub detail_dll_past_limit: &'static str,
    pub detail_dll_refused: &'static str,
    pub detail_dll_unknown: &'static str,
    pub detail_elevated: &'static str,
    pub detail_not_elevated: &'static str,
    pub update_available_title: &'static str,
    pub update_available_body: &'static str,
    pub update_offer_title: &'static str,
    pub update_offer_body: &'static str,
    pub update_failed_title: &'static str,
    pub update_failed_body: &'static str,
    pub panel_update_line: &'static str,
    pub panel_update_download: &'static str,
    pub check_ltk: &'static str,
    pub folder_restore_mods: &'static str,
    pub custom_mods_restore_busy: &'static str,
    pub check_custom_mods: &'static str,
    pub detail_custom_mods: &'static str,
    pub custom_mods_repaired_title: &'static str,
    pub custom_mods_repaired_body: &'static str,
    pub custom_mods_refused_body: &'static str,
    pub custom_mods_restored_title: &'static str,
    pub custom_mods_restored_body: &'static str,
    pub custom_mods_restore_failed: &'static str,
    pub detail_ltk_current: &'static str,
    pub detail_ltk_unchecked: &'static str,
    pub detail_ltk_new: &'static str,
    pub detail_ltk_untrusted: &'static str,
    pub panel_ltk_missing_line: &'static str,
    pub panel_ltk_new_line: &'static str,
    pub panel_ltk_untrusted_line: &'static str,
    pub panel_ltk_download: &'static str,
    pub ltk_new_title: &'static str,
    pub ltk_new_body: &'static str,
    pub ltk_version_unknown: &'static str,
    pub panel_mark_problem: &'static str,
    pub panel_mark_problem_hint: &'static str,
    pub panel_export_diagnostics: &'static str,

    pub injector_auto_title: &'static str,
    pub injector_auto_body: &'static str,
    pub injector_auto_failed: &'static str,
    pub injector_installed_title: &'static str,
    pub injector_installed_body: &'static str,
    pub injector_install_declined: &'static str,
    pub missing_tools_title: &'static str,
    pub missing_tools_body: &'static str,
    pub broken_tools_title: &'static str,
    pub broken_tools_body: &'static str,

    pub already_running_title: &'static str,
    pub already_running_body: &'static str,

    pub party_unavailable_title: &'static str,
    pub party_unavailable_body: &'static str,
    pub party_created_title: &'static str,
    pub party_created_body: &'static str,
    pub party_copy_failed_body: &'static str,
    pub party_join_title: &'static str,
    pub party_join_empty_clipboard: &'static str,
    pub party_join_clipboard_error: &'static str,
    pub party_joining: &'static str,
    pub party_invalid_code: &'static str,
    pub party_own_room: &'static str,

    pub party_dialog_create_title: &'static str,
    pub party_dialog_create_desc: &'static str,
    pub party_dialog_join_title: &'static str,
    pub party_dialog_join_desc: &'static str,
    pub party_dialog_label_code: &'static str,
    pub party_dialog_placeholder: &'static str,
    pub party_dialog_btn_copy: &'static str,
    pub party_dialog_btn_paste: &'static str,
    pub party_dialog_btn_ok: &'static str,
    pub party_dialog_btn_join: &'static str,
    pub party_dialog_btn_cancel: &'static str,
    pub party_dialog_copied: &'static str,
    pub party_dialog_error_empty: &'static str,

    pub import_title: &'static str,
    pub import_refused: &'static str,
    pub import_unsupported_extension: &'static str,
    pub import_not_a_mod: &'static str,
    pub import_no_manifest: &'static str,
    pub import_no_content: &'static str,
    pub import_no_champion: &'static str,
    pub import_io_error: &'static str,

    pub overlay_search_skin: &'static str,
    pub overlay_search_mod: &'static str,
    pub overlay_dice: &'static str,
    pub overlay_minimize: &'static str,
    pub overlay_restore: &'static str,
    pub overlay_hide: &'static str,
    pub overlay_no_champion: &'static str,
    pub overlay_classic_suffix: &'static str,
    pub overlay_historic_tag: &'static str,
    pub overlay_random_tag: &'static str,
    pub overlay_waiting_big: &'static str,
    pub overlay_waiting_sub: &'static str,
    pub overlay_no_results_big: &'static str,
    pub overlay_no_results_sub: &'static str,
    pub overlay_no_library_big: &'static str,
    pub overlay_no_library_sub: &'static str,
    pub overlay_connected: &'static str,
    pub overlay_skin_one: &'static str,
    pub overlay_skin_many: &'static str,
    pub overlay_tab_skins: &'static str,
    pub overlay_tab_mods: &'static str,
    pub overlay_slot_skin: &'static str,
    pub overlay_slot_map: &'static str,
    pub overlay_slot_font: &'static str,
    pub overlay_slot_announcer: &'static str,
    pub overlay_slot_others: &'static str,
    pub overlay_none: &'static str,
    pub overlay_open_folder: &'static str,
    pub overlay_import_mod: &'static str,
    pub overlay_no_mods_big: &'static str,
    pub overlay_no_mods_sub: &'static str,
    pub overlay_no_mods_match: &'static str,
    pub overlay_tools_missing: &'static str,
    pub overlay_lobby_waiting_big: &'static str,
    pub overlay_lobby_waiting_sub: &'static str,
    pub overlay_lobby_champions: &'static str,
    pub overlay_pin: &'static str,
    pub overlay_unpin: &'static str,
    pub overlay_preset_tag: &'static str,
    pub overlay_profile_new: &'static str,
    pub overlay_profile_delete: &'static str,
    pub overlay_profile_default: &'static str,
    pub overlay_profile_base: &'static str,
    pub category_ui: &'static str,
    pub category_voiceover: &'static str,
    pub category_loading_screen: &'static str,
    pub category_vfx: &'static str,
    pub category_sfx: &'static str,
    pub category_other: &'static str,
    pub welcome_active: &'static str,
    pub welcome_background: &'static str,
    pub welcome_author: &'static str,
    pub welcome_tray_hint: &'static str,
    pub welcome_dismiss: &'static str,

    pub welcome_quote: &'static str,
    pub party_room_full: &'static str,

    pub about_title: &'static str,
    pub about_educational: &'static str,
    pub about_quote: &'static str,
    pub about_dismiss: &'static str,
}

#[must_use]
pub fn fill(template: &str, key: &str, value: &str) -> String {
    template.replace(&format!("{{{key}}}"), value)
}

mod en;
mod tr;

use en::ENGLISH;
use tr::TURKISH;

#[cfg(test)]
mod tests;
