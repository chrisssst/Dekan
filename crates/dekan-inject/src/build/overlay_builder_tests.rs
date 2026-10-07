use super::*;
use dekan_wad::hash::content_checksum;
use dekan_wad::writer::Payload;
use std::sync::Arc;

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dekan_native_overlay_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&path); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0); // ignore-ok: fixture cleanup
    }
}

fn raw(bytes: &[u8]) -> WriterEntry {
    WriterEntry {
        kind: 0,
        subchunk_count: 0,
        first_subchunk: 0,
        uncompressed_size: bytes.len() as u64,
        checksum: content_checksum(bytes),
        payload: Payload::Memory(Arc::from(bytes.to_vec())),
    }
}

fn write_wad(path: &Path, entries: &[(u64, &[u8])]) {
    let mut writer = WadWriter::default();
    for (hash, bytes) in entries {
        writer.insert(*hash, raw(bytes));
    }
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, writer.to_bytes().expect("wad")).expect("write");
}

fn make_mod(mods: &Path, name: &str) -> PathBuf {
    let dir = mods.join(name);
    std::fs::create_dir_all(dir.join("META")).expect("meta");
    std::fs::write(dir.join("META").join("info.json"), "{}").expect("info");
    dir
}

fn read(path: &Path, hash: u64) -> Option<Vec<u8>> {
    WadFile::open(path).expect("open").read(hash).expect("read")
}

fn game(root: &Path) -> PathBuf {
    let game = root.join("Game");
    let final_dir = game.join("DATA").join("FINAL");
    write_wad(
        &final_dir.join("Champions").join("Zed.wad.client"),
        &[
            (1, b"zed skin0"),
            (2, b"zed model"),
            (3, b"shadow skin0"),
            (5, b"shadow cape"),
        ],
    );
    write_wad(
        &final_dir.join("Champions").join("Shadow.wad.client"),
        &[(5, b"shadow cape"), (9, b"shadow model")],
    );
    write_wad(
        &final_dir
            .join("Maps")
            .join("Shipping")
            .join("Map11.wad.client"),
        &[(3, b"shadow skin0"), (10, b"map terrain")],
    );
    write_wad(
        &final_dir
            .join("Maps")
            .join("Shipping")
            .join("Map22.wad.client"),
        &[(3, b"shadow skin0")],
    );
    std::fs::write(game.join("League of Legends.exe"), b"exe").expect("exe");
    game
}

#[test]
fn test_a_mod_is_merged_into_its_wad_and_every_champion_wad_sharing_a_path() {
    let root = TempDir::new("merge");
    let game = game(&root.0);
    let mods = root.0.join("mods");
    let skin = make_mod(&mods, "zed_skin");
    write_wad(
        &skin.join("WAD").join("Zed.wad.client"),
        &[
            (1, b"new skin0"),
            (3, b"new shadow"),
            (4, b"new particle"),
            (5, b"new cape"),
        ],
    );
    let overlay = root.0.join("overlay");

    let build = build(
        &game,
        &mods,
        &overlay,
        &["zed_skin".into()],
        &AtomicBool::new(false),
    )
    .expect("build");
    assert_eq!(
        build.wad_files, 3,
        "Zed, Shadow and Map11, which share paths with the mod; TFT left out"
    );

    let zed = overlay.join("DATA/FINAL/Champions/Zed.wad.client");
    assert_eq!(read(&zed, 1).as_deref(), Some(&b"new skin0"[..]));
    assert_eq!(
        read(&zed, 2).as_deref(),
        Some(&b"zed model"[..]),
        "base entries kept"
    );
    assert_eq!(
        read(&zed, 4).as_deref(),
        Some(&b"new particle"[..]),
        "new entries added"
    );
    assert_eq!(read(&zed, 5).as_deref(), Some(&b"new cape"[..]));

    let shadow = overlay.join("DATA/FINAL/Champions/Shadow.wad.client");
    assert_eq!(
        read(&shadow, 5).as_deref(),
        Some(&b"new cape"[..]),
        "the shared path agrees"
    );
    assert_eq!(read(&shadow, 9).as_deref(), Some(&b"shadow model"[..]));
    assert_eq!(
        read(&shadow, 1),
        None,
        "only the shared entries go into the shadow WAD"
    );

    assert_eq!(read(&zed, 3).as_deref(), Some(&b"new shadow"[..]));
    let map = overlay.join("DATA/FINAL/Maps/Shipping/Map11.wad.client");
    assert_eq!(
        read(&map, 3).as_deref(),
        Some(&b"new shadow"[..]),
        "the map WAD that holds the same path agrees with the champion"
    );
    assert_eq!(
        read(&map, 10).as_deref(),
        Some(&b"map terrain"[..]),
        "the rest of the map is the game's"
    );
    assert_eq!(
        read(&map, 1),
        None,
        "only the shared entries go into the map"
    );
    assert!(
        !overlay
            .join("DATA/FINAL/Maps/Shipping/Map22.wad.client")
            .exists(),
        "TFT maps must not be dragged"
    );

    let again = super::build(
        &game,
        &mods,
        &overlay,
        &["zed_skin".into()],
        &AtomicBool::new(false),
    )
    .expect("again");
    assert_eq!(again.written, 0);
}

#[test]
fn test_no_mounted_wad_disagrees_with_another_about_a_path() {
    let root = TempDir::new("consistent");
    let game = game(&root.0);
    let mods = root.0.join("mods");
    let skin = make_mod(&mods, "zed_skin");
    write_wad(
        &skin.join("WAD").join("Zed.wad.client"),
        &[(1, b"new skin0"), (3, b"new shadow"), (5, b"new cape")],
    );
    let overlay = root.0.join("overlay");
    build(
        &game,
        &mods,
        &overlay,
        &["zed_skin".into()],
        &AtomicBool::new(false),
    )
    .expect("build");

    let mut files = Vec::new();
    collect_game_wads(&game.join("DATA").join("FINAL"), &mut files);
    let mut seen: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    for file in files {
        let relpath = file.strip_prefix(&game).expect("relative");
        let file_name = relpath
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        if TFT_MOUNTS.contains(&mount_name(&file_name).as_str()) {
            continue;
        }
        let mounted = if overlay.join(relpath).exists() {
            overlay.join(relpath)
        } else {
            file.clone()
        };
        let wad = WadFile::open(&mounted).expect("open");
        for entry in wad.toc() {
            let bytes = wad.read(entry.path_hash).expect("read").expect("present");
            if let Some(other) = seen.insert(entry.path_hash, bytes.clone()) {
                assert_eq!(
                    other,
                    bytes,
                    "path {:#x} differs between mounted WADs ({})",
                    entry.path_hash,
                    mounted.display()
                );
            }
        }
    }
}

#[test]
fn test_a_map_mod_still_rewrites_the_map_and_every_wad_sharing_its_paths() {
    let root = TempDir::new("mapmod");
    let game = game(&root.0);
    let mods = root.0.join("mods");
    let map_mod = make_mod(&mods, "rift");
    write_wad(
        &map_mod.join("WAD").join("Map11.wad.client"),
        &[(3, b"new shadow"), (10, b"new terrain")],
    );
    let overlay = root.0.join("overlay");

    let build = build(
        &game,
        &mods,
        &overlay,
        &["rift".into()],
        &AtomicBool::new(false),
    )
    .expect("build");
    assert_eq!(build.wad_files, 2, "Map11 plus Zed, which shares path 3");

    let map = overlay.join("DATA/FINAL/Maps/Shipping/Map11.wad.client");
    assert_eq!(read(&map, 10).as_deref(), Some(&b"new terrain"[..]));
    let zed = overlay.join("DATA/FINAL/Champions/Zed.wad.client");
    assert_eq!(
        read(&zed, 3).as_deref(),
        Some(&b"new shadow"[..]),
        "the champion WAD agrees with the map"
    );
}

#[test]
fn test_a_path_a_map_also_holds_changes_in_the_map_too_under_the_games_header() {
    const SHARED_WITH_MAP: u64 = 0x36c2_a502_d404_b9a7;
    let root = TempDir::new("zed238068");
    let game = root.0.join("Game");
    let final_dir = game.join("DATA").join("FINAL");

    write_wad(
        &final_dir.join("Champions").join("Zed.wad.client"),
        &[
            (0x1111, b"zed skin0 model"),
            (SHARED_WITH_MAP, b"original shared asset"),
        ],
    );
    write_wad(
        &final_dir
            .join("Maps")
            .join("Shipping")
            .join("Map11.wad.client"),
        &[
            (SHARED_WITH_MAP, b"original shared asset"),
            (0x2222, b"two gigabytes of terrain, pretend"),
        ],
    );
    std::fs::write(game.join("League of Legends.exe"), b"exe").expect("exe");

    let mods = root.0.join("mods");
    let skin = make_mod(&mods, "238_238068");
    write_wad(
        &skin.join("WAD").join("Zed.wad.client"),
        &[
            (0x1111, b"shockblade model"),
            (SHARED_WITH_MAP, b"shockblade shared asset"),
        ],
    );
    let overlay = root.0.join("overlay");
    let build = build(
        &game,
        &mods,
        &overlay,
        &["238_238068".into()],
        &AtomicBool::new(false),
    )
    .expect("build");

    assert_eq!(build.wad_files, 2, "Zed.wad.client and Map11.wad.client");
    let zed = overlay.join("DATA/FINAL/Champions/Zed.wad.client");
    let map = overlay.join("DATA/FINAL/Maps/Shipping/Map11.wad.client");
    assert_eq!(
        read(&zed, 0x1111).as_deref(),
        Some(&b"shockblade model"[..])
    );
    for wad in [&zed, &map] {
        assert_eq!(
            read(wad, SHARED_WITH_MAP).as_deref(),
            Some(&b"shockblade shared asset"[..]),
            "both mounted WADs agree on the shared path"
        );
    }
    assert_eq!(
        read(&map, 0x2222).as_deref(),
        Some(&b"two gigabytes of terrain, pretend"[..])
    );
    for (overlay_wad, game_wad) in [
        (&zed, final_dir.join("Champions").join("Zed.wad.client")),
        (
            &map,
            final_dir
                .join("Maps")
                .join("Shipping")
                .join("Map11.wad.client"),
        ),
    ] {
        let head = |path: &Path| std::fs::read(path).expect("wad")[..268].to_vec();
        assert_eq!(
            head(overlay_wad),
            head(&game_wad),
            "signature and checksum are the game's"
        );
    }
}

#[test]
fn test_a_mod_entry_identical_to_the_game_is_dropped_and_shares_nothing() {
    let root = TempDir::new("h3");
    let game = game(&root.0);
    let mods = root.0.join("mods");
    let skin = make_mod(&mods, "zed_skin");

    write_wad(
        &skin.join("WAD").join("Zed.wad.client"),
        &[(1, b"new skin0"), (5, b"shadow cape")],
    );
    let overlay = root.0.join("overlay");
    let build = build(
        &game,
        &mods,
        &overlay,
        &["zed_skin".into()],
        &AtomicBool::new(false),
    )
    .expect("build");

    assert_eq!(
        build.wad_files, 1,
        "only Zed: entry 5 is a no-op, so Shadow (which shared only entry 5) is not cloned"
    );
    let zed = overlay.join("DATA/FINAL/Champions/Zed.wad.client");
    assert_eq!(
        read(&zed, 1).as_deref(),
        Some(&b"new skin0"[..]),
        "the real change lands"
    );
    assert!(
        !overlay
            .join("DATA/FINAL/Champions/Shadow.wad.client")
            .exists(),
        "a no-op entry drags no shared WAD into the overlay"
    );
}

#[test]
fn test_game_index_is_read_again_when_a_wad_changes() {
    let root = TempDir::new("reindex");
    let game = game(&root.0);
    let first = get_or_index_game(&game).expect("index");
    assert!(!first["map11"].contains(1));
    assert!(
        Arc::ptr_eq(&first, &get_or_index_game(&game).expect("again")),
        "unchanged files reuse the index"
    );

    write_wad(
        &game.join("DATA/FINAL/Maps/Shipping/Map11.wad.client"),
        &[
            (1, b"zed skin0"),
            (3, b"shadow skin0"),
            (10, b"map terrain"),
        ],
    );
    let second = get_or_index_game(&game).expect("reindex");
    assert!(
        second["map11"].contains(1),
        "the patched WAD's names are seen"
    );
}

#[test]
fn test_loose_files_raw_hex_names_blocked_tables_and_a_later_mod_winning() {
    let root = TempDir::new("loose");
    let game = game(&root.0);
    let mods = root.0.join("mods");

    let first = make_mod(&mods, "first");
    let folder = first.join("WAD").join("Zed.wad.client");
    std::fs::create_dir_all(folder.join("data")).expect("dir");
    std::fs::write(folder.join("data").join("x.bin"), b"loose x").expect("x");
    std::fs::write(folder.join("0000000000000002.bin"), b"hex model").expect("hex");
    std::fs::write(folder.join("0000000000000001.bin"), b"first skin0").expect("skin");

    let toc = subchunk_toc_hash(Path::new("DATA/FINAL/Champions/Zed.wad.client"));
    std::fs::write(
        folder.join(format!("{toc:016x}.subchunktoc")),
        b"hostile table",
    )
    .expect("toc");

    let second = make_mod(&mods, "second");
    let raw_dir = second.join("RAW");
    std::fs::create_dir_all(&raw_dir).expect("raw");
    std::fs::write(raw_dir.join("0000000000000001.bin"), b"raw wins").expect("raw");

    let overlay = root.0.join("overlay");
    build(
        &game,
        &mods,
        &overlay,
        &["first".into(), "second".into()],
        &AtomicBool::new(false),
    )
    .expect("build");

    let zed = overlay.join("DATA/FINAL/Champions/Zed.wad.client");
    assert_eq!(
        read(&zed, 2).as_deref(),
        Some(&b"hex model"[..]),
        "hex name is the hash"
    );
    assert_eq!(
        read(&zed, wad_path_hash("data/x.bin")).as_deref(),
        Some(&b"loose x"[..]),
        "a loose file is hashed by its path"
    );
    assert_eq!(
        read(&zed, 1).as_deref(),
        Some(&b"raw wins"[..]),
        "the later mod wins"
    );
    assert_eq!(read(&zed, toc), None, "subchunk tables are blocked");
}

#[test]
fn test_strays_are_removed_and_a_mod_with_no_game_wad_is_an_error() {
    let root = TempDir::new("stray");
    let game = game(&root.0);
    let mods = root.0.join("mods");
    let overlay = root.0.join("overlay");
    let old = overlay.join("DATA/FINAL/Champions/Ahri.wad.client");
    std::fs::create_dir_all(old.parent().expect("parent")).expect("dir");
    std::fs::write(&old, b"old").expect("old");

    let skin = make_mod(&mods, "zed");
    write_wad(&skin.join("WAD").join("Zed.wad.client"), &[(1, b"z")]);
    let build = build(
        &game,
        &mods,
        &overlay,
        &["zed".into()],
        &AtomicBool::new(false),
    )
    .expect("build");
    assert_eq!(build.removed, 1);
    assert!(!old.exists(), "a WAD from an earlier build is removed");

    let orphan = make_mod(&mods, "orphan");
    write_wad(
        &orphan.join("WAD").join("Nothing.wad.client"),
        &[(777, b"x")],
    );
    let err = super::build(
        &game,
        &mods,
        &overlay,
        &["orphan".into()],
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(err.to_string().contains("no game WAD"), "{err}");

    let cancelled = super::build(
        &game,
        &mods,
        &overlay,
        &["zed".into()],
        &AtomicBool::new(true),
    )
    .unwrap_err();
    assert!(cancelled.to_string().contains("cancelled"), "{cancelled}");
}

#[test]
fn test_a_map_copy_leaves_the_served_folder_when_unused_and_comes_back_when_needed() {
    let root = TempDir::new("base_store");
    let game = game(&root.0);
    let mods = root.0.join("mods");
    let shadow = make_mod(&mods, "zed_shadow");
    write_wad(
        &shadow.join("WAD").join("Zed.wad.client"),
        &[(3, b"new shadow")],
    );
    let plain = make_mod(&mods, "zed_plain");
    write_wad(
        &plain.join("WAD").join("Zed.wad.client"),
        &[(1, b"new skin0")],
    );
    let overlay = root.0.join("Dekan").join("overlay");
    let served = overlay.join("DATA/FINAL/Maps/Shipping/Map11.wad.client");
    let kept = root
        .0
        .join("Dekan")
        .join(BASE_STORE_DIR)
        .join("DATA/FINAL/Maps/Shipping/Map11.wad.client");
    let run = |name: &str| {
        build(
            &game,
            &mods,
            &overlay,
            &[name.into()],
            &AtomicBool::new(false),
        )
        .expect("build")
    };

    run("zed_shadow");
    assert!(served.is_file() && base_stamp_path(&served).is_file());

    run("zed_plain");
    assert!(!served.exists(), "an unneeded map copy is never served");
    assert!(kept.is_file() && base_stamp_path(&kept).is_file());

    run("zed_shadow");
    assert!(served.is_file(), "the kept copy is moved back");
    assert!(!kept.exists());
    assert_eq!(read(&served, 3).as_deref(), Some(&b"new shadow"[..]));
    assert_eq!(read(&served, 10).as_deref(), Some(&b"map terrain"[..]));
}

#[test]
fn test_a_started_match_stops_the_copy_ahead_and_leaves_nothing_half_written() {
    let root = TempDir::new("prewarm_stop");
    let game = game(&root.0);
    let overlay = root.0.join("Dekan").join("overlay");
    let map11 = root
        .0
        .join("Dekan")
        .join(BASE_STORE_DIR)
        .join("DATA/FINAL/Maps/Shipping/Map11.wad.client");

    assert!(matches!(
        prewarm_shared_copies(&game, &overlay, &[3], &|| true),
        Err(InjectError::Cancelled)
    ));
    assert!(!map11.exists());
    assert!(!base_stamp_path(&map11).exists());
    let leftovers = map11
        .parent()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .map_or(0, |entries| entries.count());
    assert_eq!(leftovers, 0, "no partial copy is left behind");
}

#[test]
fn test_prewarm_copies_only_the_maps_holding_the_champions_skin_bins() {
    let root = TempDir::new("prewarm");
    let game = game(&root.0);
    let overlay = root.0.join("Dekan").join("overlay");
    let store = root.0.join("Dekan").join(BASE_STORE_DIR);
    let map11 = "DATA/FINAL/Maps/Shipping/Map11.wad.client";

    assert_eq!(
        prewarm_shared_copies(&game, &overlay, &[1, 9], &|| false).expect("none"),
        0
    );
    assert!(!store.join(map11).exists());

    assert_eq!(
        prewarm_shared_copies(&game, &overlay, &[3], &|| false).expect("map11"),
        1
    );
    assert!(store.join(map11).is_file());
    assert!(
        !store
            .join("DATA/FINAL/Maps/Shipping/Map22.wad.client")
            .exists(),
        "TFT maps are never copied"
    );
    assert_eq!(
        prewarm_shared_copies(&game, &overlay, &[3], &|| false).expect("again"),
        0,
        "a valid copy is not made twice"
    );

    let mods = root.0.join("mods");
    let skin = make_mod(&mods, "zed_shadow");
    write_wad(
        &skin.join("WAD").join("Zed.wad.client"),
        &[(3, b"new shadow")],
    );
    build(
        &game,
        &mods,
        &overlay,
        &["zed_shadow".into()],
        &AtomicBool::new(false),
    )
    .expect("build");
    assert!(
        !store.join(map11).exists(),
        "the prepared copy is moved into the overlay"
    );
    assert_eq!(
        read(&overlay.join(map11), 3).as_deref(),
        Some(&b"new shadow"[..])
    );
}

fn prop(links: &[&str], objects: Vec<(u32, Vec<dekan_wad::prop::tree::Field>)>) -> Vec<u8> {
    let entries = objects
        .into_iter()
        .enumerate()
        .map(|(key, (class_hash, fields))| dekan_wad::prop::PropEntry {
            class_hash,
            key_hash: u32::try_from(key).expect("key"),
            body: dekan_wad::prop::tree::write_fields(&fields).expect("fields"),
        })
        .collect();
    dekan_wad::prop::serialize_prop_file(&dekan_wad::prop::PropFile {
        version: 3,
        links: links.iter().map(|l| (*l).to_owned()).collect(),
        entries,
    })
    .expect("prop")
}

fn path_field(name: u32, kind: u8, path: &str) -> dekan_wad::prop::tree::Field {
    use dekan_wad::prop::tree::{FIELD_FILE, Field, Value};
    let bytes = if kind == FIELD_FILE {
        wad_path_hash(path).to_le_bytes().to_vec()
    } else {
        let mut text = u16::try_from(path.len())
            .expect("short")
            .to_le_bytes()
            .to_vec();
        text.extend_from_slice(path.as_bytes());
        text
    };
    Field {
        name,
        value: Value::Raw { kind, bytes },
    }
}

#[test]
fn test_a_stale_mod_bin_gets_the_file_references_the_installed_game_declares() {
    use dekan_wad::prop::tree::{FIELD_FILE, parse_fields};
    const STRING: u8 = 16;
    let root = TempDir::new("retype");
    let game = root.0.join("Game");
    let skin = wad_path_hash("data/characters/varus/skins/skin0.bin");
    let shared_link = "DATA/Characters/Varus/Varus_Multi.bin";
    write_wad(
        &game.join("DATA/FINAL/Champions/Varus.wad.client"),
        &[
            (
                skin,
                &prop(
                    &[shared_link],
                    vec![(
                        0x9B67,
                        vec![path_field(1, FIELD_FILE, "ASSETS/Varus/Skin.tex")],
                    )],
                ),
            ),
            (
                wad_path_hash(shared_link),
                &prop(
                    &[],
                    vec![(
                        0xBEEF,
                        vec![path_field(7, FIELD_FILE, "ASSETS/Varus/Fx.dds")],
                    )],
                ),
            ),
        ],
    );
    let mods = root.0.join("mods");
    let sniper = make_mod(&mods, "sniper");
    write_wad(
        &sniper.join("WAD").join("Varus.wad.client"),
        &[(
            skin,
            &prop(
                &[shared_link],
                vec![
                    (
                        0x9B67,
                        vec![path_field(1, STRING, "ASSETS/Sniper/Skin.tex")],
                    ),
                    (0xBEEF, vec![path_field(7, STRING, "ASSETS/Sniper/Fx.dds")]),
                    (
                        0xCAFE,
                        vec![path_field(9, STRING, "ASSETS/Sniper/Unknown.dds")],
                    ),
                ],
            ),
        )],
    );
    let overlay = root.0.join("overlay");

    build(
        &game,
        &mods,
        &overlay,
        &["sniper".into()],
        &AtomicBool::new(false),
    )
    .expect("build");

    let served =
        read(&overlay.join("DATA/FINAL/Champions/Varus.wad.client"), skin).expect("skin bin");
    let file = dekan_wad::prop::parse_prop_file(&served).expect("prop");
    let kinds: Vec<(u8, Vec<u8>)> = file
        .entries
        .iter()
        .map(|entry| {
            let fields = parse_fields(&entry.body).expect("fields");
            match &fields[0].value {
                dekan_wad::prop::tree::Value::Raw { kind, bytes } => (*kind, bytes.clone()),
                other => panic!("unexpected {other:?}"),
            }
        })
        .collect();
    assert_eq!(
        kinds[0],
        (
            FIELD_FILE,
            wad_path_hash("assets/sniper/skin.tex")
                .to_le_bytes()
                .to_vec()
        )
    );
    assert_eq!(
        kinds[1],
        (
            FIELD_FILE,
            wad_path_hash("assets/sniper/fx.dds").to_le_bytes().to_vec()
        ),
        "classes defined only in a linked bin are known too"
    );
    assert_eq!(
        kinds[2].0, STRING,
        "a class the game does not have is left as is"
    );
    assert_eq!(file.links, vec![shared_link.to_owned()]);
}

#[test]
fn test_wads_in_subfolders_and_packed_dot_wad_files_are_merged() {
    let root = TempDir::new("nested");
    let game = game(&root.0);
    let mods = root.0.join("mods");
    let skin = make_mod(&mods, "nested");
    write_wad(
        &skin.join("WAD").join("Champions").join("Zed.wad.client"),
        &[(1, b"nested skin0")],
    );
    write_wad(
        &skin.join("WAD").join("Shadow.wad"),
        &[(9, b"packed model")],
    );
    let overlay = root.0.join("overlay");

    build(
        &game,
        &mods,
        &overlay,
        &["nested".into()],
        &AtomicBool::new(false),
    )
    .expect("build");

    assert_eq!(
        read(&overlay.join("DATA/FINAL/Champions/Zed.wad.client"), 1).as_deref(),
        Some(&b"nested skin0"[..])
    );
    assert_eq!(
        read(&overlay.join("DATA/FINAL/Champions/Shadow.wad.client"), 9).as_deref(),
        Some(&b"packed model"[..])
    );
}

#[test]
fn random_mod_stacks_keep_every_overlay_consistent_with_the_game_and_the_last_mod() {
    use std::collections::{BTreeMap, BTreeSet};
    let mut state = 0x5EED_0201u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let wads = [
        ("Champions/Zed.wad.client", "Zed"),
        ("Champions/Shen.wad.client", "Shen"),
        ("Champions/Akali.wad.client", "Akali"),
        ("Maps/Shipping/Map11.wad.client", "Map11"),
    ];
    for scenario in 0..60 {
        let root = TempDir::new(&format!("stack_{scenario}"));
        let game = root.0.join("Game");
        let mut game_entries: BTreeMap<&str, BTreeMap<u64, Vec<u8>>> = BTreeMap::new();
        for (relpath, _) in wads {
            let entries: BTreeMap<u64, Vec<u8>> = (0..6 + next() % 6)
                .map(|_| {
                    let hash = 1 + next() % 40;
                    (hash, format!("game {hash}").into_bytes())
                })
                .collect();
            let refs: Vec<(u64, &[u8])> = entries.iter().map(|(h, b)| (*h, b.as_slice())).collect();
            write_wad(&game.join("DATA/FINAL").join(relpath), &refs);
            game_entries.insert(relpath, entries);
        }

        let mods = root.0.join("mods");
        let mut names = Vec::new();
        let mut last: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
        for index in 0..1 + next() % 4 {
            let name = format!("mod{index}");
            let dir = make_mod(&mods, &name);
            let target = wads[(next() % wads.len() as u64) as usize].1;
            let entries: BTreeMap<u64, Vec<u8>> = (0..1 + next() % 8)
                .map(|_| {
                    let hash = 1 + next() % 48;
                    (hash, format!("{name} {hash} {}", next() % 3).into_bytes())
                })
                .collect();
            let refs: Vec<(u64, &[u8])> = entries.iter().map(|(h, b)| (*h, b.as_slice())).collect();
            write_wad(&dir.join("WAD").join(format!("{target}.wad.client")), &refs);
            last.extend(entries);
            names.push(name);
        }

        let overlay = root.0.join("overlay");
        build(&game, &mods, &overlay, &names, &AtomicBool::new(false)).expect("build");

        let mut seen: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
        for (relpath, entries) in &game_entries {
            let served = overlay.join("DATA/FINAL").join(relpath);
            let touched: BTreeSet<u64> = entries
                .keys()
                .copied()
                .filter(|h| last.get(h).is_some_and(|b| b != &entries[h]))
                .collect();
            if !served.is_file() {
                assert!(
                    touched.is_empty(),
                    "scenario {scenario}: {relpath} holds a changed path but was not served"
                );
                continue;
            }
            let wad = WadFile::open(&served).expect("served wad");
            for (hash, original) in entries {
                let bytes = wad.read(*hash).expect("read").expect("game entry kept");
                let expected = last.get(hash).unwrap_or(original);
                assert_eq!(&bytes, expected, "scenario {scenario}: {relpath} {hash}");
            }
            for entry in wad.toc() {
                let bytes = wad.read(entry.path_hash).expect("read").expect("listed");
                if let Some(previous) = seen.insert(entry.path_hash, bytes.clone()) {
                    assert_eq!(
                        previous, bytes,
                        "scenario {scenario}: path {} differs between WADs",
                        entry.path_hash
                    );
                }
                let stored = wad.read_raw(entry).expect("raw");
                assert_eq!(
                    entry.checksum,
                    content_checksum(&stored),
                    "scenario {scenario}: checksum of {}",
                    entry.path_hash
                );
            }
        }
    }
}

#[test]
fn a_saved_game_index_comes_back_whole_and_anything_stale_or_damaged_is_refused() {
    let root = TempDir::new("index_cache");
    let game = game(&root.0);
    let mut files = Vec::new();
    collect_game_wads(&game.join("DATA").join("FINAL"), &mut files);
    files.sort();
    let fingerprint = files_fingerprint(&files);
    let index = index_game(&game, files.clone()).expect("index");
    let file = root.0.join("game_index.bin");

    store_index(&file, fingerprint, &index);
    let loaded = load_index(&file, fingerprint, &game).expect("loaded");
    assert_eq!(loaded.len(), index.len());
    for (mount, wad) in &index {
        let back = &loaded[mount];
        assert_eq!(
            (&back.relpath, &back.path, &back.names),
            (&wad.relpath, &wad.path, &wad.names)
        );
    }
    assert!(
        load_index(&file, fingerprint ^ 1, &game).is_none(),
        "another game build"
    );

    let bytes = std::fs::read(&file).expect("bytes");
    for cut in 0..bytes.len() {
        assert!(
            parse_index(&bytes[..cut], fingerprint, &game).is_none(),
            "cut at {cut}"
        );
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(
        parse_index(&longer, fingerprint, &game).is_none(),
        "trailing bytes"
    );

    std::thread::sleep(std::time::Duration::from_millis(20));
    write_wad(&files[0], &[(77, b"patched")]);
    assert_ne!(
        files_fingerprint(&files),
        fingerprint,
        "a patched WAD changes the fingerprint"
    );
}
