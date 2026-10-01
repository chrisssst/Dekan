use std::path::Path;

use dekan_core::historic::HistoricBook;
use dekan_platform::fs::atomic_write;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

const HISTORIC_FILE: &str = "historic.json";

const HISTORIC_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct PersistedHistoric {
    version: u32,
    book: HistoricBook,
}

#[must_use]
pub fn load(state_dir: &Path) -> HistoricBook {
    let path = state_dir.join(HISTORIC_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return HistoricBook::default(),
        Err(e) => {
            warn!(file = %path.display(), error = %e, "Historic skins could not be read; starting empty");
            return HistoricBook::default();
        }
    };

    match serde_json::from_slice::<PersistedHistoric>(&bytes) {
        Ok(persisted) => {
            if persisted.version != HISTORIC_VERSION {
                warn!(
                    file = %path.display(),
                    version = persisted.version,
                    expected = HISTORIC_VERSION,
                    "Historic skins written by another version; reading them as-is"
                );
            }
            info!(champions = persisted.book.len(), "Historic skins restored");
            persisted.book
        }
        Err(e) => {
            let aside = path.with_extension("json.unreadable");
            let moved = std::fs::rename(&path, &aside);
            warn!(
                file = %path.display(),
                moved_to = %aside.display(),
                moved = moved.is_ok(),
                error = %e,
                "Historic skins file is not valid; it was set aside and the book starts empty"
            );
            HistoricBook::default()
        }
    }
}

pub fn save(state_dir: &Path, book: &HistoricBook) {
    let path = state_dir.join(HISTORIC_FILE);
    let persisted = PersistedHistoric {
        version: HISTORIC_VERSION,
        book: book.clone(),
    };
    let bytes = match serde_json::to_vec_pretty(&persisted) {
        Ok(bytes) => bytes,
        Err(e) => {
            error!(error = %e, "Historic skins could not be serialized; they will not survive a restart");
            return;
        }
    };
    match atomic_write(&path, &bytes, true) {
        Ok(()) => debug!(file = %path.display(), "Historic skins saved"),
        Err(e) => warn!(file = %path.display(), error = %e, "Historic skins could not be saved"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dekan_core::overlay::OverlayTarget;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dekan_historic_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: test scratch folder may not exist
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn a_saved_book_loads_back() {
        let dir = scratch("roundtrip");
        let mut book = HistoricBook::default();
        book.record(&OverlayTarget {
            champion_id: 238,
            skin_id: 238068,
            chroma_id: None,
        });
        save(&dir, &book);
        assert_eq!(load(&dir), book);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_is_an_empty_book() {
        let dir = scratch("missing");
        assert!(load(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corrupt_file_is_set_aside_not_overwritten() {
        let dir = scratch("corrupt");
        std::fs::write(dir.join(HISTORIC_FILE), b"{not json").expect("write");
        assert!(load(&dir).is_empty());
        assert!(dir.join("historic.json.unreadable").exists());
        assert!(!dir.join(HISTORIC_FILE).exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
