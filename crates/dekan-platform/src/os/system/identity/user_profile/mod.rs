use std::path::PathBuf;
use std::sync::OnceLock;

use crate::error::PlatformError;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use macos::resolve;
#[cfg(windows)]
use windows::resolve;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    SameUser { path: PathBuf },

    DesktopUser { path: PathBuf, own: Option<PathBuf> },

    OwnProfile { path: PathBuf, reason: String },

    Unresolved { reason: String },
}

impl Resolution {
    fn path(&self) -> Option<&PathBuf> {
        match self {
            Self::SameUser { path }
            | Self::DesktopUser { path, .. }
            | Self::OwnProfile { path, .. } => Some(path),
            Self::Unresolved { .. } => None,
        }
    }
}

static RESOLUTION: OnceLock<Resolution> = OnceLock::new();

pub fn resolution() -> &'static Resolution {
    RESOLUTION.get_or_init(resolve)
}

pub fn local_app_data() -> Result<PathBuf, PlatformError> {
    match resolution() {
        Resolution::Unresolved { reason } => Err(PlatformError::Path(format!(
            "LOCALAPPDATA could not be resolved: {reason}"
        ))),
        resolved => resolved
            .path()
            .cloned()
            .ok_or_else(|| PlatformError::Path("LOCALAPPDATA could not be resolved".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_folder_resolves_to_an_existing_directory() {
        match resolution() {
            Resolution::Unresolved { reason } => panic!("unresolved: {reason}"),
            resolved => {
                let path = resolved.path().expect("resolved path");
                assert!(path.is_dir(), "{} is not a directory", path.display());
            }
        }
        assert!(local_app_data().is_ok());
    }
}
