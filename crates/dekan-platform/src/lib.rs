#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod activation;
pub mod autostart;
pub mod client_window;
pub mod clipboard;
pub mod dialog;
pub mod elevation;
pub mod error;
pub mod fs;
pub mod game_version;
pub mod hotkey;
pub mod i18n;
pub mod overlay_window;
pub mod panel;
pub mod party_dialog;
pub mod paths;
pub mod preferences;
pub mod process;
pub mod shell;
pub mod single_instance;
pub mod tray;
pub mod user_profile;
pub mod version;
pub mod welcome;

#[cfg(test)]
mod ui_pages_dump;
