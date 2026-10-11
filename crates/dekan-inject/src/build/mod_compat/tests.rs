use super::*;
use dekan_wad::prop::serialize_prop_links;
use dekan_wad::writer::{WadWriter, WriterEntry};

pub(super) fn raw(bytes: &[u8]) -> WriterEntry {
    dekan_wad::writer::optimal_raw(bytes.to_vec()).expect("raw")
}

pub(super) fn write_wad(path: &Path, entries: &[(u64, Vec<u8>)]) {
    let mut writer = WadWriter::default();
    for (hash, bytes) in entries {
        writer.insert(*hash, raw(bytes));
    }
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, writer.to_bytes().expect("wad")).expect("write");
}

pub(super) fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dekan_modcompat_{name}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

pub(super) fn relink_wad(
    wad: &Path,
    game: &BTreeMap<String, GameWad>,
    hashes: &HashSet<u64>,
) -> Result<Vec<Relink>, InjectError> {
    let own = mod_hashes(&[wad.to_path_buf()]);
    Repairer::new(game, hashes)
        .repair(wad, &own)
        .map(|r| r.relinks)
}

pub(super) fn check_wad(wad: &Path, hashes: &HashSet<u64>) -> Result<ModCompat, InjectError> {
    check(wad, hashes, &HashSet::new())
}

pub(super) fn shared(slots: &[u32], root: bool) -> String {
    let mut parts: Vec<String> = slots.iter().map(|n| format!("Skins_Skin{n}")).collect();
    parts.sort();
    let root = if root { "Root_" } else { "" };
    format!(
        "DATA/Characters/Zed/Zed_Multi_Skins_{root}{}.bin",
        parts.join("_")
    )
}

pub(super) fn game_with(dir: &Path, skin0_links: &[String]) -> BTreeMap<String, GameWad> {
    let path = dir.join("game").join("Zed.wad.client");
    let mut entries = vec![(
        wad_path_hash("data/characters/zed/skins/skin0.bin"),
        serialize_prop_links(skin0_links, 3),
    )];
    for link in skin0_links {
        entries.push((
            wad_path_hash(&link.to_ascii_lowercase()),
            b"shared".to_vec(),
        ));
    }
    write_wad(&path, &entries);
    let mut names: Vec<u64> = entries.iter().map(|(h, _)| *h).collect();
    names.sort_unstable();
    BTreeMap::from([(
        "zed".to_owned(),
        GameWad {
            relpath: PathBuf::from("DATA/FINAL/Champions/Zed.wad.client"),
            path,
            names,
        },
    )])
}

pub(super) fn mod_with(dir: &Path, links: &[String]) -> (PathBuf, dekan_wad::prop::PropFile) {
    let prop = dekan_wad::prop::PropFile {
        version: 3,
        links: links.to_vec(),
        entries: vec![dekan_wad::prop::PropEntry {
            class_hash: 0x1111,
            key_hash: 0x2222,
            body: vec![0, 0],
        }],
    };
    let wad = dir.join("mod").join("Zed.wad.client");
    write_wad(
        &wad,
        &[
            (
                wad_path_hash("data/characters/zed/skins/skin0.bin"),
                serialize_prop_file(&prop).expect("prop"),
            ),
            (wad_path_hash("assets/zed/new.tex"), b"texture".to_vec()),
        ],
    );
    (wad, prop)
}

pub(super) fn skn(major: u16, minor: u16) -> Vec<u8> {
    let mut head = vec![0x33, 0x22, 0x11, 0x00];
    head.extend_from_slice(&major.to_le_bytes());
    head.extend_from_slice(&minor.to_le_bytes());
    head.resize(32, 0);
    head
}

#[test]
fn test_shared_skin_bins_renamed_by_a_new_skin_are_relinked() {
    let dir = temp("relink");
    let base = "DATA/Characters/Zed/Zed.bin".to_owned();
    let game = game_with(
        &dir,
        &[
            shared(&[0, 1, 2, 44], false),
            base.clone(),
            shared(&[0, 1, 44], true),
            shared(&[0, 1, 44], false),
        ],
    );
    let (wad, before) = mod_with(
        &dir,
        &[
            base.clone(),
            shared(&[0, 1], false),
            shared(&[0, 1, 2], false),
            shared(&[0, 1], true),
        ],
    );
    let mut hashes = game_hash_set(&game);
    hashes.insert(wad_path_hash(&base.to_ascii_lowercase()));

    let relinks = relink_wad(&wad, &game, &hashes).expect("relink");
    assert_eq!(relinks.len(), 3, "{relinks:?}");
    assert!(check_wad(&wad, &hashes).expect("check").is_compatible());

    let after = WadFile::open(&wad).expect("open");
    let prop = parse_prop_file(
        &after
            .read(wad_path_hash("data/characters/zed/skins/skin0.bin"))
            .expect("read")
            .expect("skin0"),
    )
    .expect("prop");
    assert_eq!(
        prop.links,
        vec![
            base,
            shared(&[0, 1, 44], false),
            shared(&[0, 1, 2, 44], false),
            shared(&[0, 1, 44], true),
        ],
        "each link keeps its place"
    );
    assert_eq!(prop.entries, before.entries, "objects are untouched");
    assert_eq!(
        after
            .read(wad_path_hash("assets/zed/new.tex"))
            .expect("read"),
        Some(b"texture".to_vec())
    );
    assert!(!wad.with_extension("relinked").exists());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_legacy_shared_skin_bin_is_relinked_to_its_characters_folder() {
    let dir = temp("legacy");
    let game = game_with(
        &dir,
        &[
            shared(&[0, 1], false),
            shared(&[0, 1, 44], false),
            shared(&[0, 1], true),
        ],
    );
    let (wad, _) = mod_with(
        &dir,
        &[
            "DATA/Zed_Skins_Skin0_Skins_Skin1.bin".to_owned(),
            "DATA/Zed_Skins_Root_Skins_Skin0_Skins_Skin1.bin".to_owned(),
        ],
    );
    let hashes = game_hash_set(&game);

    let relinks = relink_wad(&wad, &game, &hashes).expect("relink");
    assert_eq!(
        relinks.iter().map(|r| r.to.clone()).collect::<Vec<_>>(),
        vec![shared(&[0, 1], false), shared(&[0, 1], true)],
        "the same slots win over a group that gained a skin"
    );
    assert!(check_wad(&wad, &hashes).expect("check").is_compatible());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

pub(super) fn string_field(name: u32, text: &str) -> dekan_wad::prop::tree::Field {
    let mut bytes = u16::try_from(text.len())
        .expect("len")
        .to_le_bytes()
        .to_vec();
    bytes.extend_from_slice(text.as_bytes());
    dekan_wad::prop::tree::Field {
        name,
        value: Value::Raw {
            kind: FIELD_STRING,
            bytes,
        },
    }
}

pub(super) fn strings_of(prop: &dekan_wad::prop::PropFile) -> Vec<String> {
    let fields = parse_fields(&prop.entries[0].body).expect("fields");
    fields
        .iter()
        .filter_map(|f| match &f.value {
            Value::Raw { bytes, .. } => Some(String::from_utf8_lossy(&bytes[2..]).to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn test_an_unpacked_wad_folder_is_checked_and_repaired_in_place() {
    let dir = temp("folder");
    let game = game_with(&dir, &[shared(&[0, 1, 44], false)]);
    let hashes = game_hash_set(&game);
    let folder = dir.join("mod").join("WAD").join("Zed.wad.client");
    let bin = folder.join("data/characters/zed/skins/skin0.bin");
    std::fs::create_dir_all(bin.parent().expect("parent")).expect("dir");
    std::fs::write(&bin, serialize_prop_links(&[shared(&[0, 1], false)], 3)).expect("bin");

    let wads = mod_wads(&dir.join("mod"));
    assert_eq!(wads, vec![folder.clone()]);
    let own = mod_hashes(&wads);
    assert!(
        !check(&folder, &hashes, &own)
            .expect("check")
            .is_compatible()
    );

    let repair = Repairer::new(&game, &hashes)
        .repair(&folder, &own)
        .expect("repair");
    assert_eq!(repair.files, vec![bin.clone()]);
    assert!(
        check(&folder, &hashes, &own)
            .expect("check")
            .is_compatible()
    );
    assert_eq!(
        parse_prop_links(&std::fs::read(&bin).expect("read")).expect("links"),
        vec![shared(&[0, 1, 44], false)]
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_successor_the_bin_already_links_is_refused() {
    let dir = temp("duplicate");
    let game = game_with(&dir, &[shared(&[0, 1, 44], false)]);
    let (wad, _) = mod_with(&dir, &[shared(&[0, 1], false), shared(&[0, 1, 44], false)]);
    let hashes = game_hash_set(&game);

    assert!(relink_wad(&wad, &game, &hashes).expect("relink").is_empty());
    assert!(!check_wad(&wad, &hashes).expect("check").is_compatible());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_an_ambiguous_successor_leaves_the_mod_untouched() {
    let dir = temp("ambiguous");
    let game = game_with(
        &dir,
        &[shared(&[0, 1, 44], false), shared(&[0, 1, 45], false)],
    );
    let (wad, _) = mod_with(&dir, &[shared(&[0, 1], false)]);
    let original = std::fs::read(&wad).expect("read");
    let hashes = game_hash_set(&game);

    assert!(relink_wad(&wad, &game, &hashes).expect("relink").is_empty());
    assert_eq!(std::fs::read(&wad).expect("read"), original);
    assert!(!check_wad(&wad, &hashes).expect("check").is_compatible());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_successor_that_adds_a_slot_the_mod_already_knew_is_refused() {
    let dir = temp("known_slot");
    let game = game_with(&dir, &[shared(&[0, 1, 2], false)]);
    let (wad, _) = mod_with(&dir, &[shared(&[0, 1], false), shared(&[2, 3], false)]);
    let hashes = game_hash_set(&game);

    assert!(relink_wad(&wad, &game, &hashes).expect("relink").is_empty());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_mod_without_dangling_links_is_not_rewritten() {
    let dir = temp("clean");
    let link = shared(&[0, 1, 44], false);
    let game = game_with(&dir, std::slice::from_ref(&link));
    let (wad, _) = mod_with(&dir, &[link]);
    let original = std::fs::read(&wad).expect("read");

    let relinks = relink_wad(&wad, &game, &game_hash_set(&game)).expect("relink");
    assert!(relinks.is_empty());
    assert_eq!(std::fs::read(&wad).expect("read"), original);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}
