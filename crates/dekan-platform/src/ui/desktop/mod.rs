#[cfg(windows)]
pub mod clipboard;
#[cfg(windows)]
pub mod dialog;
#[cfg(windows)]
pub mod hotkey;
#[cfg(windows)]
pub mod shell;
pub mod tray;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{clipboard, dialog, hotkey, shell};

#[must_use]
pub fn is_web_page(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty()
            && !rest.starts_with('/')
            && url
                .chars()
                .all(|c| c.is_ascii_graphic() && c != '"' && c != '\\')
    })
}
