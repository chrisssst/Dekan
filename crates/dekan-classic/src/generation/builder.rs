pub const CLASSIC_CHAMPION_OFFSET: u32 = 60_000;

pub const CLASSIC_SKIN_OFFSET: u32 = 60_000_000;

pub const CLASSIC_DEFAULT_SLOTS: [u32; 3] = [0, 301, 302];

#[must_use]
pub fn is_classic_champion(id: u32) -> bool {
    (CLASSIC_CHAMPION_OFFSET..CLASSIC_SKIN_OFFSET).contains(&id)
}

#[must_use]
pub fn normalize_champion_id(id: u32) -> u32 {
    if is_classic_champion(id) {
        id - CLASSIC_CHAMPION_OFFSET
    } else {
        id
    }
}

#[must_use]
pub fn normalize_skin_id(id: u32) -> u32 {
    id.checked_sub(CLASSIC_SKIN_OFFSET).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_ids_map_back_to_the_regular_ones_and_regular_ids_stay() {
        assert!(is_classic_champion(60_012));
        assert!(!is_classic_champion(12));
        assert!(!is_classic_champion(60_012_001));
        assert_eq!(normalize_champion_id(60_012), 12);
        assert_eq!(normalize_champion_id(12), 12);
        assert_eq!(normalize_skin_id(60_012_001), 12_001);
        assert_eq!(normalize_skin_id(12_001), 12_001);
    }
}
