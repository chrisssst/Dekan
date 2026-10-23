#[cfg(windows)]
pub mod activation;
#[cfg(windows)]
pub mod single_instance;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{activation, single_instance};
