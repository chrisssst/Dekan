#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod league;
mod os;
mod ui;

pub use league::{client_settings, client_window, game_version, paths};
pub use os::{
    activation, authenticode, autostart, elevation, fs, preferences, process, single_instance,
    user_profile, version,
};
pub use ui::{
    clipboard, dialog, hotkey, i18n, overlay_window, panel, party_dialog, shell, tray, welcome,
};

pub mod error;
