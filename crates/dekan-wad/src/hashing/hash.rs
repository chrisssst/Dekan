use xxhash_rust::xxh3::xxh3_64;
use xxhash_rust::xxh64::xxh64;

#[must_use]
pub fn wad_path_hash(path: &str) -> u64 {
    let lower = path.to_ascii_lowercase();
    xxh64(lower.as_bytes(), 0)
}

#[must_use]
pub fn relative_path_hash(relative: &str) -> u64 {
    let trimmed = relative.trim_start_matches(['.', '/']);
    let stem = trimmed.split('.').next().unwrap_or(trimmed);

    if stem.len() == 16 && stem.bytes().all(|b| b.is_ascii_hexdigit()) {
        if let Ok(literal) = u64::from_str_radix(stem, 16) {
            return literal;
        }
    }
    wad_path_hash(trimmed)
}

#[must_use]
pub fn mount_name(file_name: &str) -> String {
    let mut name = file_name.to_ascii_lowercase();
    if let Some(stripped) = name.strip_suffix(".client") {
        name.truncate(stripped.len());
    }
    if let Some(stripped) = name.strip_suffix(".wad") {
        name.truncate(stripped.len());
    }
    name
}

#[must_use]
pub fn prop_key_hash(key: &str) -> u32 {
    const FNV_OFFSET_BASIS: u32 = 0x811C_9DC5;
    const FNV_PRIME: u32 = 0x0100_0193;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in key.bytes() {
        let b = byte.to_ascii_lowercase();
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[must_use]
pub fn content_checksum(data: &[u8]) -> u64 {
    xxh3_64(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wad_path_hash_case_insensitivity() {
        let h1 = wad_path_hash("DATA/Characters/Annie/Skins/Skin0.bin");
        let h2 = wad_path_hash("data/characters/annie/skins/skin0.bin");
        let h3 = wad_path_hash("DaTa/ChArAcTeRs/AnNiE/sKiNs/SkIn0.bIn");
        assert_eq!(h1, h2);
        assert_eq!(h2, h3);
        assert_ne!(h1, 0);
    }

    #[test]
    fn test_relative_path_hash_follows_mkoverlay() {
        let path = "data/characters/zed/skins/skin0.bin";
        assert_eq!(relative_path_hash(path), wad_path_hash(path));
        assert_eq!(
            relative_path_hash("DATA/Characters/Zed/Skins/Skin0.bin"),
            wad_path_hash(path)
        );
        assert_eq!(
            relative_path_hash("./data/x.bin"),
            wad_path_hash("data/x.bin")
        );

        assert_eq!(
            relative_path_hash("0123456789abcdef.bin"),
            0x0123_4567_89ab_cdef
        );
        assert_eq!(
            relative_path_hash("0123456789ABCDEF"),
            0x0123_4567_89ab_cdef
        );

        assert_eq!(
            relative_path_hash("0123456789abcdeg.bin"),
            wad_path_hash("0123456789abcdeg.bin")
        );
        assert_eq!(
            relative_path_hash("sub/0123456789abcdef.bin"),
            wad_path_hash("sub/0123456789abcdef.bin")
        );
        assert_eq!(
            relative_path_hash("+123456789abcdef"),
            wad_path_hash("+123456789abcdef")
        );
    }

    #[test]
    fn test_mount_name_follows_mkoverlay() {
        assert_eq!(mount_name("Zed.wad.client"), "zed");
        assert_eq!(mount_name("Zed.pt_BR.wad.client"), "zed.pt_br");
        assert_eq!(mount_name("Map11.WAD"), "map11");
        assert_eq!(mount_name("_RAW.wad.client"), "_raw");
        assert_eq!(mount_name("readme.txt"), "readme.txt");
    }

    #[test]
    fn test_prop_key_hash_case_insensitivity() {
        let h1 = prop_key_hash("Characters/Annie/Skins/Skin0");
        let h2 = prop_key_hash("characters/annie/skins/skin0");
        assert_eq!(h1, h2);
        assert_ne!(h1, 0);
    }

    #[test]
    fn test_content_checksum() {
        let data = b"League of Legends asset data";
        let c1 = content_checksum(data);
        let c2 = content_checksum(data);
        assert_eq!(c1, c2);
        assert_ne!(c1, 0);
    }
}
