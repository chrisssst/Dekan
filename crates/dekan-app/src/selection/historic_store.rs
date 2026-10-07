use std::path::Path;

use dekan_core::historic::HistoricBook;
use tracing::info;

use crate::book_store;

const HISTORIC_FILE: &str = "historic.json";

#[must_use]
pub fn load(state_dir: &Path) -> HistoricBook {
    let book: HistoricBook = book_store::load(&state_dir.join(HISTORIC_FILE), "historic skins");
    if !book.is_empty() {
        info!(champions = book.len(), "Historic skins restored");
    }
    book
}

pub fn save(state_dir: &Path, book: &HistoricBook) {
    book_store::save(&state_dir.join(HISTORIC_FILE), book, "historic skins");
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
