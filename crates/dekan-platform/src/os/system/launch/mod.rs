#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod elevation;
#[cfg(windows)]
pub mod process;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{autostart, elevation, process};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevatedRun {
    Finished(u32),
    Declined,
}
