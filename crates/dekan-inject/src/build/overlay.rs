use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use crate::error::InjectError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayConfig {
    pub mods_dir: PathBuf,

    pub overlay_dir: PathBuf,

    pub game_dir: PathBuf,
}

pub struct OverlayManager;

impl OverlayManager {
    pub fn prepare_overlay_dir(overlay_dir: &Path) -> Result<(), InjectError> {
        if !overlay_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(overlay_dir) {
                warn!(
                    overlay_dir = %overlay_dir.display(),
                    error = %e,
                    "Could not create the overlay directory"
                );
                return Err(InjectError::Io(e));
            }
            debug!(overlay_dir = %overlay_dir.display(), "Overlay directory created");
        }
        Ok(())
    }
}
