use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::historic::HistoricEntry;
use crate::overlay::OverlayTarget;
use crate::selection::ChampionId;

pub const DEFAULT_PROFILE: &str = "";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetBook {
    #[serde(default)]
    active: String,
    #[serde(default)]
    profiles: BTreeMap<String, BTreeMap<ChampionId, HistoricEntry>>,
}

impl PresetBook {
    #[must_use]
    pub fn active(&self) -> &str {
        if self.profiles.contains_key(&self.active) {
            &self.active
        } else {
            DEFAULT_PROFILE
        }
    }

    #[must_use]
    pub fn profiles(&self) -> Vec<String> {
        std::iter::once(DEFAULT_PROFILE.to_owned())
            .chain(
                self.profiles
                    .keys()
                    .filter(|name| name.as_str() != DEFAULT_PROFILE)
                    .cloned(),
            )
            .collect()
    }

    #[must_use]
    pub fn preset(&self, champion_id: ChampionId) -> Option<HistoricEntry> {
        self.profiles
            .get(self.active())
            .and_then(|presets| presets.get(&champion_id))
            .copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.profiles.values().map(BTreeMap::len).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn toggle(&mut self, target: &OverlayTarget) -> bool {
        let entry = HistoricEntry {
            skin_id: target.skin_id,
            chroma_id: target.chroma_id,
        };
        let active = self.active().to_owned();
        let presets = self.profiles.entry(active).or_default();
        if presets.get(&target.champion_id) == Some(&entry) {
            presets.remove(&target.champion_id);
            false
        } else {
            presets.insert(target.champion_id, entry);
            true
        }
    }

    pub fn switch_to(&mut self, name: &str) -> bool {
        if name != DEFAULT_PROFILE && !self.profiles.contains_key(name) {
            return false;
        }
        if self.active() == name {
            return false;
        }
        self.active = name.to_owned();
        true
    }

    pub fn create(&mut self, base: &str) -> String {
        let name = (2..)
            .map(|n| format!("{base} {n}"))
            .find(|candidate| !self.profiles.contains_key(candidate))
            .unwrap_or_else(|| base.to_owned());
        self.profiles.insert(name.clone(), BTreeMap::new());
        self.active = name.clone();
        name
    }

    pub fn delete_active(&mut self) -> Option<String> {
        let active = self.active().to_owned();
        if active == DEFAULT_PROFILE {
            return None;
        }
        self.profiles.remove(&active);
        self.active = DEFAULT_PROFILE.to_owned();
        Some(active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(champion_id: ChampionId, skin_id: u32, chroma_id: Option<u32>) -> OverlayTarget {
        OverlayTarget {
            champion_id,
            skin_id,
            chroma_id,
        }
    }

    #[test]
    fn pinning_the_same_skin_twice_unpins_it_and_another_skin_replaces_it() {
        let mut book = PresetBook::default();
        assert!(book.toggle(&target(238, 238_012, None)));
        assert_eq!(
            book.preset(238).map(|e| e.package_entry_id()),
            Some(238_012)
        );

        assert!(book.toggle(&target(238, 238_001, Some(238_005))));
        assert_eq!(
            book.preset(238).map(|e| e.package_entry_id()),
            Some(238_005)
        );

        assert!(!book.toggle(&target(238, 238_001, Some(238_005))));
        assert!(book.preset(238).is_none());
    }

    #[test]
    fn each_profile_keeps_its_own_presets() {
        let mut book = PresetBook::default();
        book.toggle(&target(238, 238_012, None));

        let ranked = book.create("Profile");
        assert_eq!(ranked, "Profile 2");
        assert_eq!(book.active(), "Profile 2");
        assert!(book.preset(238).is_none(), "a new profile starts empty");
        book.toggle(&target(238, 238_070, None));

        assert!(book.switch_to(DEFAULT_PROFILE));
        assert_eq!(book.preset(238).map(|e| e.skin_id), Some(238_012));
        assert!(book.switch_to("Profile 2"));
        assert_eq!(book.preset(238).map(|e| e.skin_id), Some(238_070));

        assert_eq!(book.create("Profile"), "Profile 3");
        assert_eq!(
            book.profiles(),
            vec![
                String::new(),
                "Profile 2".to_owned(),
                "Profile 3".to_owned()
            ]
        );
        assert!(!book.switch_to("missing"));
    }

    #[test]
    fn deleting_a_profile_falls_back_to_the_default_which_cannot_be_deleted() {
        let mut book = PresetBook::default();
        assert_eq!(book.delete_active(), None);
        let name = book.create("Profile");
        book.toggle(&target(103, 103_015, None));
        assert_eq!(book.delete_active(), Some(name));
        assert_eq!(book.active(), DEFAULT_PROFILE);
        assert!(book.preset(103).is_none());
    }

    #[test]
    fn the_book_round_trips_through_json_and_tolerates_an_unknown_active_profile() {
        let mut book = PresetBook::default();
        book.toggle(&target(238, 238_012, None));
        book.create("Ranked");
        book.toggle(&target(145, 145_005, Some(145_012)));
        let json = serde_json::to_string(&book).expect("serialize");
        let back: PresetBook = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, book);

        let stale: PresetBook =
            serde_json::from_str(r#"{"active":"gone","profiles":{}}"#).expect("deserialize");
        assert_eq!(stale.active(), DEFAULT_PROFILE);
        assert!(
            serde_json::from_str::<PresetBook>("{}")
                .expect("empty")
                .is_empty()
        );
    }
}
