use crate::error::ClassicError;

pub const CLASSIC_CHAMPION_OFFSET: u32 = 60_000;

pub const CLASSIC_SKIN_OFFSET: u32 = 60_000_000;

pub const CLASSIC_DEFAULT_SLOTS: [u32; 3] = [0, 301, 302];

pub struct ClassicIdMapper;

impl ClassicIdMapper {
    #[must_use]
    pub fn is_classic_champion(id: u32) -> bool {
        (CLASSIC_CHAMPION_OFFSET..CLASSIC_SKIN_OFFSET).contains(&id)
    }

    #[must_use]
    pub fn is_classic_skin(id: u32) -> bool {
        id >= CLASSIC_SKIN_OFFSET
    }

    #[must_use]
    pub fn normalize_champion_id(id: u32) -> u32 {
        if (CLASSIC_CHAMPION_OFFSET..CLASSIC_SKIN_OFFSET).contains(&id) {
            id - CLASSIC_CHAMPION_OFFSET
        } else {
            id
        }
    }

    #[must_use]
    pub fn to_classic_champion_id(id: u32) -> u32 {
        if id < CLASSIC_CHAMPION_OFFSET {
            id + CLASSIC_CHAMPION_OFFSET
        } else {
            id
        }
    }

    #[must_use]
    pub fn normalize_skin_id(id: u32) -> u32 {
        if id >= CLASSIC_SKIN_OFFSET {
            id - CLASSIC_SKIN_OFFSET
        } else {
            id
        }
    }

    #[must_use]
    pub fn to_classic_skin_id(id: u32) -> u32 {
        if id < CLASSIC_SKIN_OFFSET {
            id + CLASSIC_SKIN_OFFSET
        } else {
            id
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicModConfig {
    pub champion_id: u32,

    pub skin_id: u32,

    pub target_slots: Vec<u32>,

    pub bypass_base_skin_force: bool,
}

impl ClassicModConfig {
    pub fn new(raw_champ_id: u32, raw_skin_id: u32) -> Result<Self, ClassicError> {
        let champion_id = ClassicIdMapper::normalize_champion_id(raw_champ_id);
        let skin_id = ClassicIdMapper::normalize_skin_id(raw_skin_id);

        if champion_id == 0 {
            return Err(ClassicError::ChampionNotFound {
                alias: "Invalid champion ID 0".into(),
            });
        }

        let selected_slot = skin_id % 1000;
        let mut target_slots = CLASSIC_DEFAULT_SLOTS.to_vec();
        if !target_slots.contains(&selected_slot) {
            target_slots.push(selected_slot);
        }

        Ok(Self {
            champion_id,
            skin_id,
            target_slots,
            bypass_base_skin_force: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classic_id_mapper_roundtrip() {
        let original_champ = 12;
        let classic_champ = ClassicIdMapper::to_classic_champion_id(original_champ);
        assert_eq!(classic_champ, 60012);
        assert!(ClassicIdMapper::is_classic_champion(classic_champ));
        assert_eq!(
            ClassicIdMapper::normalize_champion_id(classic_champ),
            original_champ
        );

        let original_skin = 12001;
        let classic_skin = ClassicIdMapper::to_classic_skin_id(original_skin);
        assert_eq!(classic_skin, 60012001);
        assert!(ClassicIdMapper::is_classic_skin(classic_skin));
        assert_eq!(
            ClassicIdMapper::normalize_skin_id(classic_skin),
            original_skin
        );
    }

    #[test]
    fn test_classic_mod_config_slots_and_no_base_force() {
        let config = ClassicModConfig::new(60012, 60012301).expect("valid classic config");
        assert_eq!(config.champion_id, 12);
        assert_eq!(config.skin_id, 12301);
        assert!(
            config.bypass_base_skin_force,
            "must not force base skin in Classic"
        );
        assert!(config.target_slots.contains(&0));
        assert!(config.target_slots.contains(&301));
        assert!(config.target_slots.contains(&302));

        let config2 = ClassicModConfig::new(60012, 60012001).expect("valid classic config");
        assert!(
            config2.target_slots.contains(&1),
            "slot 1 should be included in target slots"
        );
    }
}
