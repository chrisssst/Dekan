#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{seal, unseal};
#[cfg(windows)]
pub use windows::{seal, unseal};

#[cfg(test)]
mod tests;
