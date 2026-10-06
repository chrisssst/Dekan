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
fn test_prewarm_copies_only_the_maps_holding_the_champions_skin_bins() {
    let root = TempDir::new("prewarm");
    let game = game(&root.0);
    let overlay = root.0.join("Dekan").join("overlay");
    let store = root.0.join("Dekan").join(BASE_STORE_DIR);
    let map11 = "DATA/FINAL/Maps/Shipping/Map11.wad.client";

    assert_eq!(
        prewarm_shared_copies(&game, &overlay, &[1, 9]).expect("none"),
        0
    );
    assert!(!store.join(map11).exists());

    assert_eq!(
        prewarm_shared_copies(&game, &overlay, &[3]).expect("map11"),
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
        prewarm_shared_copies(&game, &overlay, &[3]).expect("again"),
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
