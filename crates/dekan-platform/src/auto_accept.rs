//! "Accept match automatically" preference, toggled from the tray.
//!
//! Stored as a small JSON file in Dekan's state folder so it survives restarts. Off by default:
//! accepting a match on the user's behalf is something they opt into.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::PlatformError;

const FILE_NAME: &str = "auto_accept.json";

static ENABLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    enabled: bool,
}

/// Read the stored preference once at startup. A missing or unreadable file means off.
pub fn load() -> bool {
    let enabled = match crate::paths::state_dir() {
        Ok(dir) => read_from(&dir.join(FILE_NAME)),
        Err(e) => {
            warn!(error = %e, "State folder unavailable; automatic match accept stays off");
            false
        }
    };
    ENABLED.store(enabled, Ordering::Relaxed);
    enabled
}

#[must_use]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn toggle() -> Result<bool, PlatformError> {
    let next = !is_enabled();
    write_to(&crate::paths::state_dir()?.join(FILE_NAME), next)?;
    ENABLED.store(next, Ordering::Relaxed);
    Ok(next)
}

fn read_from(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Stored>(&bytes) {
            Ok(stored) => stored.enabled,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Automatic accept setting unreadable; treated as off");
                false
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Automatic accept setting could not be read; treated as off");
            false
        }
    }
}

fn write_to(path: &Path, enabled: bool) -> Result<(), PlatformError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| PlatformError::Io {
            context: format!("failed to create {}", parent.display()),
            source: e,
        })?;
    }
    let json = serde_json::to_vec(&Stored { enabled })
        .map_err(|e| PlatformError::Path(format!("automatic accept setting: {e}")))?;
    crate::fs::atomic_write(path, &json, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("dekan_auto_accept_{name}_{}", std::process::id()))
            .join(FILE_NAME)
    }

    #[test]
    fn test_a_missing_file_means_off() {
        assert!(!read_from(&temp_file("missing")));
    }

    #[test]
    fn test_the_setting_round_trips() {
        let path = temp_file("roundtrip");
        write_to(&path, true).expect("write on");
        assert!(read_from(&path));
        write_to(&path, false).expect("write off");
        assert!(!read_from(&path));
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir); // ignore-ok: fixture cleanup
        }
    }

    #[test]
    fn test_garbage_means_off() {
        let path = temp_file("garbage");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("fixture dir");
        }
        std::fs::write(&path, b"not json").expect("write garbage");
        assert!(!read_from(&path));
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir); // ignore-ok: fixture cleanup
        }
    }
}
