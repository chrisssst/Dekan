use std::path::Path;

use dekan_core::presets::PresetBook;
use tracing::info;

use crate::book_store;

const PRESETS_FILE: &str = "presets.json";

#[must_use]
pub fn load(state_dir: &Path) -> PresetBook {
    let book: PresetBook = book_store::load(&state_dir.join(PRESETS_FILE), "skin presets");
    if !book.is_empty() {
        info!(
            presets = book.len(),
            profiles = book.profiles().len(),
            active_profile = book.active(),
            "Skin presets restored"
        );
    }
    book
}

pub fn save(state_dir: &Path, book: &PresetBook) {
    book_store::save(&state_dir.join(PRESETS_FILE), book, "skin presets");
}

#[cfg(test)]
mod tests {
    use super::*;
    use dekan_core::overlay::OverlayTarget;

    #[test]
    fn saved_presets_and_the_active_profile_load_back() {
        let dir = std::env::temp_dir().join(format!("dekan_presets_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: test scratch folder may not exist
        std::fs::create_dir_all(&dir).expect("scratch dir");

        let mut book = PresetBook::default();
        book.create("Ranked");
        book.toggle(&OverlayTarget {
            champion_id: 238,
            skin_id: 238_012,
            chroma_id: None,
        });
        save(&dir, &book);
        let back = load(&dir);
        assert_eq!(back, book);
        assert_eq!(back.active(), "Ranked 2");
        std::fs::remove_dir_all(&dir).ok();
    }
}
