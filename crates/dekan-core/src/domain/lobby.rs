use serde::{Deserialize, Serialize};

use crate::overlay::OverlayTarget;
use crate::selection::{ChampionId, SkinId};

pub const LOBBY_PICK_QUEUES: [u32; 2] = [480, 490];

pub const LOBBY_PICK_MODES: [&str; 2] = ["SWIFTPLAY", "BRAWL"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySlot {
    pub champion_id: ChampionId,
    pub skin_id: SkinId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyPicks {
    pub slots: Vec<LobbySlot>,
    pub targets: Vec<OverlayTarget>,
}

impl LobbyPicks {
    #[must_use]
    pub fn champions(&self) -> Vec<ChampionId> {
        self.slots.iter().map(|slot| slot.champion_id).collect()
    }

    #[must_use]
    pub fn holds(&self, champion_id: ChampionId) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.champion_id == champion_id)
    }

    #[must_use]
    pub fn skin_of(&self, champion_id: ChampionId) -> Option<SkinId> {
        self.slots
            .iter()
            .find(|slot| slot.champion_id == champion_id)
            .map(|slot| slot.skin_id)
            .filter(|skin| *skin > 0)
    }

    #[must_use]
    pub fn target_for(&self, champion_id: ChampionId) -> Option<&OverlayTarget> {
        self.targets.iter().find(|t| t.champion_id == champion_id)
    }

    pub fn remember(&mut self, target: &OverlayTarget) {
        if !self.holds(target.champion_id) {
            return;
        }
        self.targets.retain(|t| t.champion_id != target.champion_id);
        self.targets.push(target.clone());
    }

    pub fn forget(&mut self, champion_id: ChampionId) {
        self.targets.retain(|t| t.champion_id != champion_id);
    }

    pub fn replace_slots(&mut self, slots: Vec<LobbySlot>) {
        self.slots = slots;
        let slots = &self.slots;
        self.targets
            .retain(|t| slots.iter().any(|slot| slot.champion_id == t.champion_id));
    }

    #[must_use]
    pub fn chosen_in_slot_order(&self) -> Vec<&OverlayTarget> {
        self.slots
            .iter()
            .filter_map(|slot| self.target_for(slot.champion_id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(champion_id: ChampionId, skin_id: SkinId) -> OverlayTarget {
        OverlayTarget {
            champion_id,
            skin_id,
            chroma_id: None,
        }
    }

    fn slot(champion_id: ChampionId) -> LobbySlot {
        LobbySlot {
            champion_id,
            skin_id: champion_id * 1000,
        }
    }

    #[test]
    fn a_skin_is_remembered_only_for_a_champion_in_the_lobby() {
        let mut picks = LobbyPicks {
            slots: vec![slot(238), slot(103)],
            targets: Vec::new(),
        };
        picks.remember(&target(238, 238_012));
        picks.remember(&target(1, 1_001));
        assert_eq!(picks.targets, vec![target(238, 238_012)]);

        picks.remember(&target(238, 238_070));
        assert_eq!(
            picks.target_for(238),
            Some(&target(238, 238_070)),
            "a new pick replaces the old one"
        );
    }

    #[test]
    fn swapping_a_champion_out_drops_its_skin_and_order_follows_the_slots() {
        let mut picks = LobbyPicks {
            slots: vec![slot(238), slot(103)],
            targets: Vec::new(),
        };
        picks.remember(&target(103, 103_015));
        picks.remember(&target(238, 238_012));
        assert_eq!(
            picks.chosen_in_slot_order(),
            vec![&target(238, 238_012), &target(103, 103_015)]
        );

        picks.replace_slots(vec![slot(238), slot(84)]);
        assert_eq!(picks.targets, vec![target(238, 238_012)]);
        assert_eq!(picks.champions(), vec![238, 84]);
    }

    #[test]
    fn the_slot_skin_is_reported_only_when_set() {
        let picks = LobbyPicks {
            slots: vec![
                LobbySlot {
                    champion_id: 238,
                    skin_id: 238_012,
                },
                LobbySlot {
                    champion_id: 103,
                    skin_id: 0,
                },
            ],
            targets: Vec::new(),
        };
        assert_eq!(picks.skin_of(238), Some(238_012));
        assert_eq!(picks.skin_of(103), None);
        assert_eq!(picks.skin_of(1), None);
    }
}
