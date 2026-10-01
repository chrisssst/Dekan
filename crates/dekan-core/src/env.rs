pub const LOG: &str = "DEKAN_LOG";

pub const RELAY_URL: &str = "DEKAN_RELAY_URL";

pub const PATCHER_FLAGS: &str = "DEKAN_PATCHER_FLAGS";

pub const SKIN_SYNC: &str = "DEKAN_SKIN_SYNC";

pub const ALL: [&str; 4] = [LOG, RELAY_URL, PATCHER_FLAGS, SKIN_SYNC];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_dekan_prefixed_and_unique() {
        for name in ALL {
            assert!(name.starts_with("DEKAN_"), "{name} is not DEKAN_-prefixed");
        }
        let mut seen = std::collections::HashSet::new();
        for name in ALL {
            assert!(seen.insert(name), "{name} listed twice");
        }
    }
}
