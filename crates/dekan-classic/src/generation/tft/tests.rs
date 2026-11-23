use dekan_wad::hash::{prop_key_hash, wad_path_hash};
use dekan_wad::prop::{PropEntry, PropFile, parse_prop_file, serialize_prop_file};
use dekan_wad::writer::{WadWriter, optimal_raw};

use super::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "dekan_tft_test_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0); // ignore-ok: best-effort test cleanup
    }
}

fn sample_prop(record_key: &str) -> Vec<u8> {
    let file = PropFile {
        version: 2,
        links: vec!["characters/pettftavatar/skins/root.bin".into()],
        entries: vec![PropEntry {
            class_hash: prop_key_hash("SkinCharacterDataProperties"),
            key_hash: prop_key_hash(record_key),
            body: vec![0, 0, 0, 0],
        }],
    };
    serialize_prop_file(&file).expect("valid prop")
}

fn write_test_wad(path: &Path, entries: Vec<(&str, Vec<u8>)>) {
    let mut writer = WadWriter::default();
    for (p, data) in entries {
        writer.insert(wad_path_hash(p), optimal_raw(data).expect("encode"));
    }
    writer.write_to_file(path, &|| false).expect("write wad");
}

#[test]
fn builds_tft_mod_remapping_tactician_boom_portal_and_arena() {
    let dir = TempDir::new("full");
    let game_dir = dir.path().join("Game");
    let mods_dir = dir.path().join("mods");

    let target_tactician = "data/characters/petpenguknight/skins/skin12.bin";
    let target_boom = "data/loadouts/tftdamageskins/boom_star.bin";
    let target_portal = "data/loadouts/tftzoomskins/portal_void.bin";
    let target_arena = "data/maps/shipping/map22/arena_celestial.materials.bin";

    write_test_wad(
        &game_dir.join("DATA/FINAL/Companions.wad.client"),
        vec![(
            target_tactician,
            sample_prop("characters/petpenguknight/skins/skin12"),
        )],
    );
    write_test_wad(
        &game_dir.join("DATA/FINAL/Global.wad.client"),
        vec![
            (
                target_boom,
                sample_prop("loadouts/tftdamageskins/boom_star"),
            ),
            (
                target_portal,
                sample_prop("loadouts/tftzoomskins/portal_void"),
            ),
        ],
    );
    write_test_wad(
        &game_dir.join("DATA/FINAL/Maps/Shipping/Map22.wad.client"),
        vec![(
            target_arena,
            sample_prop("maps/shipping/map22/arena_celestial.materials"),
        )],
    );

    let loadout = TftLoadout {
        tactician: Some(TftSwap {
            donor_id: 1,
            donor_path: DEFAULT_TACTICIAN_DONOR.into(),
            target_id: 12,
            target_path: target_tactician.into(),
            name: "Chibi Pengu".into(),
        }),
        boom: Some(TftSwap {
            donor_id: 1,
            donor_path: "data/loadouts/tftdamageskins/base.bin".into(),
            target_id: 99,
            target_path: target_boom.into(),
            name: "Star Boom".into(),
        }),
        portal: Some(TftSwap {
            donor_id: 1,
            donor_path: "data/loadouts/tftzoomskins/base.bin".into(),
            target_id: 42,
            target_path: target_portal.into(),
            name: "Void Portal".into(),
        }),
        arena: Some(TftSwap {
            donor_id: 1,
            donor_path: "data/maps/shipping/map22/base.materials.bin".into(),
            target_id: 77,
            target_path: target_arena.into(),
            name: "Celestial Arena".into(),
        }),
    };

    let built = build_tft_mod(&game_dir, &mods_dir, &loadout).expect("build ok");
    assert_eq!(built.as_deref(), Some(TFT_MOD_FOLDER));

    let mod_root = mods_dir.join(TFT_MOD_FOLDER);
    assert!(mod_root.join("META/info.json").is_file());

    let remapped = std::fs::read(mod_root.join("RAW").join(DEFAULT_TACTICIAN_DONOR))
        .expect("remapped tactician bin");
    let parsed = parse_prop_file(&remapped).expect("valid remapped prop");
    assert_eq!(
        parsed.entries[0].key_hash,
        prop_key_hash("characters/pettftavatar/skins/skin1")
    );
    assert!(
        mod_root
            .join("RAW/data/loadouts/tftdamageskins/base.bin")
            .is_file()
    );
    assert!(
        mod_root
            .join("RAW/data/loadouts/tftzoomskins/base.bin")
            .is_file()
    );
    assert!(
        mod_root
            .join("RAW/data/maps/shipping/map22/base.materials.bin")
            .is_file()
    );
}

#[test]
fn rejects_path_traversal_and_cleans_empty_loadout() {
    let dir = TempDir::new("hostile");
    let game_dir = dir.path().join("Game");
    let mods_dir = dir.path().join("mods");

    let hostile = TftLoadout {
        tactician: Some(TftSwap {
            donor_id: 1,
            donor_path: "../outside.bin".into(),
            target_id: 2,
            target_path: "data/characters/pet/skins/skin1.bin".into(),
            name: "Hostile".into(),
        }),
        ..TftLoadout::default()
    };
    let built = build_tft_mod(&game_dir, &mods_dir, &hostile).expect("no error");
    assert_eq!(built, None);
    assert!(!mods_dir.join(TFT_MOD_FOLDER).exists());
}
