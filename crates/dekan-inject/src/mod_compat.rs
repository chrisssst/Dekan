use std::collections::HashSet;
use std::path::Path;

use dekan_wad::hash::wad_path_hash;
use dekan_wad::prop::parse_prop_links;
use dekan_wad::wad::WadFile;
use tracing::debug;

use crate::error::InjectError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModCompat {
    pub dangling: Vec<String>,

    pub props_checked: usize,

    pub entries_skipped: usize,
}

impl ModCompat {
    #[must_use]
    pub fn is_compatible(&self) -> bool {
        self.dangling.is_empty()
    }
}

fn is_bin_link(link: &str) -> bool {
    link.rsplit('.')
        .next()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bin"))
}

pub fn check_wad(mod_wad: &Path, game_hashes: &HashSet<u64>) -> Result<ModCompat, InjectError> {
    let wad = WadFile::open(mod_wad).map_err(|e| {
        InjectError::Overlay(format!("mod WAD unreadable '{}': {e}", mod_wad.display()))
    })?;

    let own: HashSet<u64> = wad.toc().map(|e| e.path_hash).collect();
    let resolvable = |hash: u64| game_hashes.contains(&hash) || own.contains(&hash);

    let mut compat = ModCompat::default();
    let hashes: Vec<u64> = wad.toc().map(|e| e.path_hash).collect();
    for hash in hashes {
        let Ok(Some(bytes)) = wad.read(hash) else {
            compat.entries_skipped += 1;
            continue;
        };
        let Ok(links) = parse_prop_links(&bytes) else {
            compat.entries_skipped += 1;
            continue;
        };
        compat.props_checked += 1;
        for link in links {
            if !is_bin_link(&link) {
                continue;
            }

            if !resolvable(wad_path_hash(&link)) {
                debug!(
                    wad = %mod_wad.display(),
                    link = %link,
                    "Dangling PROP link: the target .bin is in neither the game nor the mod"
                );
                compat.dangling.push(link);
            }
        }
    }
    Ok(compat)
}

#[must_use]
pub fn game_hash_set(
    game: &std::collections::BTreeMap<String, crate::overlay_builder::GameWad>,
) -> HashSet<u64> {
    game.values()
        .flat_map(|wad| wad.names.iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dekan_wad::prop::serialize_prop_links;
    use dekan_wad::writer::{WadWriter, WriterEntry};

    fn raw(bytes: &[u8]) -> WriterEntry {
        dekan_wad::writer::optimal_raw(bytes.to_vec()).expect("raw")
    }

    fn write_wad(path: &Path, entries: &[(u64, Vec<u8>)]) {
        let mut writer = WadWriter::default();
        for (hash, bytes) in entries {
            writer.insert(*hash, raw(bytes));
        }
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(path, writer.to_bytes().expect("wad")).expect("write");
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dekan_modcompat_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    #[test]
    fn test_is_bin_link() {
        assert!(is_bin_link("DATA/Characters/Zed/Skins/Skin1.bin"));
        assert!(is_bin_link("thing.BIN"));
        assert!(!is_bin_link("assets/x.tex"));
        assert!(!is_bin_link("no_extension"));
    }

    #[test]
    fn test_a_link_resolved_by_the_game_or_the_mod_is_not_dangling() {
        let dir = temp("ok");
        let game_link = "DATA/Characters/Zed/Skins/Skin1.bin";
        let mod_link = "DATA/Characters/Zed/NewAsset.bin";

        let prop = serialize_prop_links(&[game_link.to_owned(), mod_link.to_owned()], 3);
        let wad = dir.join("Zed.wad.client");
        write_wad(
            &wad,
            &[
                (wad_path_hash("data/characters/zed/skins/skin0.bin"), prop),
                (wad_path_hash(mod_link), b"the mod's own target".to_vec()),
            ],
        );

        let mut game = HashSet::new();
        game.insert(wad_path_hash(game_link));

        let compat = check_wad(&wad, &game).expect("check");
        assert!(compat.is_compatible(), "{compat:?}");
        assert_eq!(compat.props_checked, 1);
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
    }

    #[test]
    fn test_a_bin_link_in_neither_game_nor_mod_is_dangling() {
        let dir = temp("dangling");
        let missing = "DATA/Characters/Zed/Skins/Skin-1.bin";
        let tex_missing = "assets/removed.tex";
        let prop = serialize_prop_links(&[missing.to_owned(), tex_missing.to_owned()], 3);
        let wad = dir.join("Zed.wad.client");
        write_wad(
            &wad,
            &[(wad_path_hash("data/characters/zed/skins/skin0.bin"), prop)],
        );

        let mut game = HashSet::new();
        game.insert(wad_path_hash("data/characters/zed/skins/skin5.bin"));

        let compat = check_wad(&wad, &game).expect("check");
        assert!(!compat.is_compatible());
        assert_eq!(
            compat.dangling,
            vec![missing.to_owned()],
            "only the .bin, not the .tex"
        );
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
    }

    #[test]
    fn test_a_non_prop_entry_is_skipped_never_flagged() {
        let dir = temp("nonprop");
        let wad = dir.join("Zed.wad.client");
        write_wad(
            &wad,
            &[(wad_path_hash("data/x.tex"), b"\x89PNG not a prop".to_vec())],
        );
        let compat = check_wad(&wad, &HashSet::new()).expect("check");
        assert!(
            compat.is_compatible(),
            "an unreadable-as-PROP entry is not damage"
        );
        assert_eq!(compat.props_checked, 0);
        assert_eq!(compat.entries_skipped, 1);
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
    }
}
