use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

pub(super) const FILE_NAME: &str = dekan_platform::paths::STAR_GATE_FILE;
pub(super) const PURPOSE: &str = "dekan-star-gate-v1";
pub(super) const CLOCK_SKEW_SECS: u64 = 300;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Grant {
    pub login: String,
    pub user_id: u64,
    pub token: String,
    pub verified_at: u64,
}

impl Grant {
    pub(super) fn is_from_the_future(&self, now: u64) -> bool {
        self.verified_at > now.saturating_add(CLOCK_SKEW_SECS)
    }
}

fn path(state_dir: &Path) -> PathBuf {
    state_dir.join(FILE_NAME)
}

pub(super) fn load(state_dir: &Path) -> Option<Grant> {
    let file = path(state_dir);
    let sealed = match std::fs::read(&file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!("StarGate has no saved verification yet");
            return None;
        }
        Err(e) => {
            warn!(path = %file.display(), error = %e, "StarGate could not read its saved verification");
            return None;
        }
    };
    let plain = match dekan_platform::vault::unseal(&sealed, PURPOSE) {
        Ok(plain) => plain,
        Err(e) => {
            warn!(path = %file.display(), error = %e, "StarGate verification belongs to another user or was altered; it is ignored");
            return None;
        }
    };
    match serde_json::from_slice(&plain) {
        Ok(grant) => Some(grant),
        Err(e) => {
            warn!(path = %file.display(), error = %e, "StarGate verification is unreadable; it is ignored");
            None
        }
    }
}

pub(super) fn save(state_dir: &Path, grant: &Grant) -> anyhow::Result<()> {
    let plain = serde_json::to_vec(grant)?;
    let sealed = dekan_platform::vault::seal(&plain, PURPOSE)?;
    dekan_platform::fs::atomic_write(&path(state_dir), &sealed, true)?;
    info!("StarGate verification saved");
    Ok(())
}

pub(super) fn forget(state_dir: &Path) {
    let file = path(state_dir);
    match std::fs::remove_file(&file) {
        Ok(()) => info!("StarGate verification removed"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            warn!(path = %file.display(), error = %e, "StarGate could not remove its saved verification");
        }
    }
}
