pub const LOG: &str = "DEKAN_LOG";

pub const RELAY_URL: &str = "DEKAN_RELAY_URL";

pub const PATCHER_FLAGS: &str = "DEKAN_PATCHER_FLAGS";

pub const SKIN_SYNC: &str = "DEKAN_SKIN_SYNC";

pub const UPDATE_CHECK: &str = "DEKAN_UPDATE_CHECK";

pub const SKIN_GRAPH: &str = "DEKAN_SKIN_GRAPH";

pub const CHROMA_CLASSIFICATION: &str = "DEKAN_CHROMA_CLASSIFICATION";

pub const ALL: [&str; 7] = [
    LOG,
    RELAY_URL,
    PATCHER_FLAGS,
    SKIN_SYNC,
    UPDATE_CHECK,
    SKIN_GRAPH,
    CHROMA_CLASSIFICATION,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_dekan_prefixed_and_unique() {
        for name in ALL {
            assert!(
                name.starts_with("DEKAN_"),
                "{name} is not DEKAN_-prefixed"
            );
        }
        let mut seen = std::collections::HashSet::new();
        for name in ALL {
            assert!(seen.insert(name), "{name} listed twice");
        }
    }
}
