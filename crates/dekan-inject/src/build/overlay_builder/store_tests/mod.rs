use super::tests::{TempDir, game, make_mod, read, write_wad};
use super::*;

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
