use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::overlay::OverlayTarget;
use crate::selection::{ChampionId, ChromaId, SelectionMode, SkinId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricEntry {
    pub skin_id: SkinId,
    #[serde(default)]
    pub chroma_id: Option<ChromaId>,
}

impl HistoricEntry {
    #[must_use]
    pub fn package_entry_id(&self) -> u32 {
        self.chroma_id.unwrap_or(self.skin_id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricBook {
    #[serde(default)]
    entries: BTreeMap<ChampionId, HistoricEntry>,
}

impl HistoricBook {
    #[must_use]
    pub fn get(&self, champion_id: ChampionId) -> Option<HistoricEntry> {
        self.entries.get(&champion_id).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn record(&mut self, target: &OverlayTarget) -> bool {
        let entry = HistoricEntry {
            skin_id: target.skin_id,
            chroma_id: target.chroma_id,
        };
        self.entries.insert(target.champion_id, entry) != Some(entry)
    }

    pub fn forget(&mut self, champion_id: ChampionId) -> bool {
        self.entries.remove(&champion_id).is_some()
    }
}

#[must_use]
pub fn may_restore(
    champion_id: ChampionId,
    has_overlay_target: bool,
    lcu_skin: Option<SkinId>,
) -> bool {
    !has_overlay_target
        && lcu_skin.is_none_or(|skin| SelectionMode::is_base_skin(skin, champion_id))
}

#[must_use]
pub fn superseded_in_client(champion_id: ChampionId, lcu_skin: Option<SkinId>) -> bool {
    lcu_skin.is_some_and(|skin| !SelectionMode::is_base_skin(skin, champion_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(
        champion_id: ChampionId,
        skin_id: SkinId,
        chroma_id: Option<ChromaId>,
    ) -> OverlayTarget {
        OverlayTarget {
            champion_id,
            skin_id,
            chroma_id,
        }
    }

    #[test]
    fn a_recorded_injection_is_what_gets_restored() {
        let mut book = HistoricBook::default();
        assert!(book.record(&target(238, 238068, None)));
        assert!(book.record(&target(145, 145005, Some(145012))));

        assert_eq!(book.get(238).map(|e| e.package_entry_id()), Some(238068));
        assert_eq!(book.get(145).map(|e| e.package_entry_id()), Some(145012));
        assert_eq!(book.get(1), None);
    }

    #[test]
    fn recording_the_same_pick_again_changes_nothing() {
        let mut book = HistoricBook::default();
        assert!(book.record(&target(238, 238068, None)));
        assert!(!book.record(&target(238, 238068, None)));

        assert!(book.record(&target(238, 238068, Some(238069))));
    }

    #[test]
    fn a_dismissed_pick_is_forgotten() {
        let mut book = HistoricBook::default();
        book.record(&target(238, 238068, None));
        assert!(book.forget(238));
        assert!(!book.forget(238));
        assert!(book.is_empty());
    }

    #[test]
    fn restore_only_over_nothing_and_over_the_base_skin() {
        assert!(may_restore(238, false, None));
        assert!(may_restore(238, false, Some(238000)));
        assert!(may_restore(238, false, Some(0)));

        assert!(!may_restore(238, false, Some(238001)));

        assert!(!may_restore(238, true, None));

        assert!(may_restore(60238, false, Some(60238000)));
    }

    #[test]
    fn a_client_side_choice_supersedes_the_restored_pick() {
        assert!(superseded_in_client(238, Some(238001)));
        assert!(!superseded_in_client(238, Some(238000)));
        assert!(!superseded_in_client(238, None));
    }

    #[test]
    fn the_book_round_trips_through_json() {
        let mut book = HistoricBook::default();
        book.record(&target(238, 238068, None));
        book.record(&target(145, 145005, Some(145012)));
        let json = serde_json::to_string(&book).expect("serialize");
        let back: HistoricBook = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, book);

        let empty: HistoricBook = serde_json::from_str("{}").expect("empty");
        assert!(empty.is_empty());
    }
}
