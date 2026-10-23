#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub(super) use macos::{hide, prepare, show_at};
#[cfg(windows)]
pub(super) use windows::{hide, prepare, show_at};
