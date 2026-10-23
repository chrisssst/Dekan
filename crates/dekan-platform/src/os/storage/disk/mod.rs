#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::disk_kind;
#[cfg(windows)]
pub use windows::disk_kind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskKind {
    SolidState,
    Rotational,
    Unknown,
}

impl DiskKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SolidState => "ssd",
            Self::Rotational => "hdd",
            Self::Unknown => "unknown",
        }
    }
}
