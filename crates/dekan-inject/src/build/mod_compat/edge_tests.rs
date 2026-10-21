use super::tests::*;
use super::*;
use dekan_wad::prop::serialize_prop_links;

struct EveryLevel;

impl tracing::Subscriber for EveryLevel {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

fn zed_game(dir: &Path, entries: &[(u64, Vec<u8>)]) -> BTreeMap<String, GameWad> {
    let path = dir.join("game").join("Zed.wad.client");
    write_wad(&path, entries);
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

fn skin0_hash() -> u64 {
    wad_path_hash("data/characters/zed/skins/skin0.bin")
}

#[test]
fn test_listing_helpers_tolerate_missing_and_broken_inputs() {
    let dir = temp("listing");
    std::fs::create_dir_all(dir.join("no_wad_here")).expect("dir");
    assert!(mod_wads(&dir.join("no_wad_here")).is_empty());
    assert!(files_under(&dir.join("does_not_exist")).is_err());
    let garbage = dir.join("Broken.wad.client");
    std::fs::write(&garbage, b"not a wad").expect("garbage");
    assert!(mod_hashes(std::slice::from_ref(&garbage)).is_empty());
    assert!(check(&garbage, &HashSet::new(), &HashSet::new()).is_err());
    let game = zed_game(&dir, &[]);
    let hashes = game_hash_set(&game);
    assert!(
        Repairer::new(&game, &hashes)
            .repair(&garbage, &HashSet::new())
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_folder_check_skips_what_is_not_a_property_file_and_logs_dangling_links() {
    let dir = temp("folder_check");
    let folder = dir.join("Zed.wad.client");
    std::fs::create_dir_all(folder.join("data")).expect("dir");
    std::fs::write(folder.join("data/broken.bin"), b"not a property file").expect("bin");
    std::fs::write(
        folder.join("data/skin0.bin"),
        serialize_prop_links(&["DATA/Gone.bin".to_owned()], 3),
    )
    .expect("bin");
    let compat = tracing::subscriber::with_default(EveryLevel, || {
        let span = tracing::info_span!("check", wad = tracing::field::Empty);
        span.record("wad", "Zed");
        span.follows_from(tracing::Span::none());
        span.follows_from(&tracing::info_span!("earlier"));
        let _entered = span.enter();
        check(&folder, &HashSet::new(), &HashSet::new()).expect("check")
    });
    assert_eq!(compat.entries_skipped, 1);
    assert_eq!(compat.dangling, vec!["DATA/Gone.bin".to_owned()]);
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_name_rules_reject_what_they_cannot_read() {
    assert!(shared_bin("DATA/Characters/Zed/Zed_Multi_Skins_Root.bin").is_none());
    assert_eq!(without_variant_suffix("Ring.Chroma"), Some("Ring"));
    assert!(asset_successors("assets/ção.tex").is_empty());
    assert!(asset_successors("assets/no_extension").is_empty());
    assert_eq!(
        asset_successors("assets/plain.tex"),
        vec!["assets/plain.dds"]
    );
}

#[test]
fn test_unreadable_or_oversized_strings_and_other_values_are_left_alone() {
    let mut not_utf8 = Value::Raw {
        kind: FIELD_STRING,
        bytes: vec![1, 0, 0xFF],
    };
    let mut number = Value::Raw {
        kind: 7,
        bytes: vec![1, 0, 0, 0],
    };
    let long = format!("assets/{}.Variant.tex", "x".repeat(70_000));
    let mut oversized = Value::Raw {
        kind: FIELD_STRING,
        bytes: [vec![0, 0], long.into_bytes()].concat(),
    };
    let mut relinks = Vec::new();
    for value in [&mut not_utf8, &mut number, &mut oversized] {
        relink_assets(value, &|_| false, &|_| true, &mut relinks);
    }
    assert!(relinks.is_empty());
}

#[test]
fn test_a_game_skin_bin_that_cannot_be_read_offers_no_successor() {
    let dir = temp("game_unreadable");
    let game = zed_game(&dir, &[(skin0_hash(), b"not a property file".to_vec())]);
    let hashes = game_hash_set(&game);
    let (wad, _) = mod_with(&dir, &[shared(&[0, 1], false)]);
    assert!(relink_wad(&wad, &game, &hashes).expect("repair").is_empty());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_formats_are_not_judged_without_a_matching_game_archive_and_fail_on_a_broken_mod() {
    let dir = temp("formats_edges");
    let game = zed_game(&dir, &[(1, skn(4, 1))]);
    let hashes = game_hash_set(&game);
    let other = dir.join("mod").join("Ahri.wad.client");
    write_wad(&other, &[(10, skn(1, 1))]);
    let mut repairer = Repairer::new(&game, &hashes);
    assert!(
        repairer
            .unknown_formats(&other)
            .expect("formats")
            .is_empty()
    );
    let broken = dir.join("broken").join("Zed.wad.client");
    std::fs::create_dir_all(broken.parent().expect("parent")).expect("dir");
    std::fs::write(&broken, b"not a wad").expect("garbage");
    assert!(repairer.unknown_formats(&broken).is_err());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_property_files_that_cannot_be_repaired_safely_are_left_alone() {
    let dir = temp("prop_edges");
    let current = shared(&[0, 1, 44], false);
    let game = game_with(&dir, std::slice::from_ref(&current));
    let hashes = game_hash_set(&game);
    let folder = dir.join("mod").join("WAD").join("Zed.wad.client");
    std::fs::create_dir_all(folder.join("data")).expect("dir");
    std::fs::write(folder.join("data/not_prop.bin"), b"PTCH not a prop").expect("bin");
    std::fs::write(folder.join("data/broken_prop.bin"), b"PROPbroken").expect("bin");
    std::fs::write(folder.join("data/texture.tex"), b"texture").expect("tex");
    let unknown_link =
        serialize_prop_links(&["DATA/Characters/Zed/Skins/Skin99.bin".to_owned()], 3);
    std::fs::write(folder.join("data/unknown_link.bin"), unknown_link).expect("bin");
    std::fs::write(
        folder.join("data/twice.bin"),
        serialize_prop_links(&[shared(&[0, 1], false), shared(&[0, 1], false)], 3),
    )
    .expect("bin");
    let unreadable_body = dekan_wad::prop::PropFile {
        version: 3,
        links: vec![current.clone()],
        entries: vec![dekan_wad::prop::PropEntry {
            class_hash: 1,
            key_hash: 2,
            body: vec![0xFF],
        }],
    };
    std::fs::write(
        folder.join("data/unreadable_body.bin"),
        serialize_prop_file(&unreadable_body).expect("prop"),
    )
    .expect("bin");
    let mut trailing = write_fields(&[string_field(1, "ASSETS/Gone.Variant.tex")]).expect("body");
    trailing.push(0);
    let not_round_trip = dekan_wad::prop::PropFile {
        version: 3,
        links: vec![current],
        entries: vec![dekan_wad::prop::PropEntry {
            class_hash: 1,
            key_hash: 3,
            body: trailing,
        }],
    };
    std::fs::write(
        folder.join("data/not_round_trip.bin"),
        serialize_prop_file(&not_round_trip).expect("prop"),
    )
    .expect("bin");

    let own = mod_hashes(std::slice::from_ref(&folder));
    let repair = Repairer::new(&game, &hashes)
        .repair(&folder, &own)
        .expect("repair");
    assert!(repair.relinks.is_empty(), "{:?}", repair.relinks);
    assert!(repair.files.is_empty());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_folder_bin_that_cannot_be_rewritten_fails_the_repair() {
    let dir = temp("folder_readonly");
    let game = game_with(&dir, &[shared(&[0, 1, 44], false)]);
    let hashes = game_hash_set(&game);
    let folder = dir.join("mod").join("WAD").join("Zed.wad.client");
    let bin = folder.join("data/characters/zed/skins/skin0.bin");
    std::fs::create_dir_all(bin.parent().expect("parent")).expect("dir");
    std::fs::write(&bin, serialize_prop_links(&[shared(&[0, 1], false)], 3)).expect("bin");
    let mut permissions = std::fs::metadata(&bin).expect("meta").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&bin, permissions.clone()).expect("readonly");

    let own = mod_hashes(std::slice::from_ref(&folder));
    assert!(Repairer::new(&game, &hashes).repair(&folder, &own).is_err());
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&bin, permissions).expect("writable");
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_a_wad_that_cannot_be_replaced_fails_and_leaves_no_temporary_file() {
    let dir = temp("wad_readonly");
    let game = game_with(&dir, &[shared(&[0, 1, 44], false)]);
    let hashes = game_hash_set(&game);
    let (wad, _) = mod_with(&dir, &[shared(&[0, 1], false)]);
    let mut permissions = std::fs::metadata(&wad).expect("meta").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&wad, permissions.clone()).expect("readonly");

    assert!(relink_wad(&wad, &game, &hashes).is_err());
    assert!(!wad.with_extension("relinked").exists());
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&wad, permissions).expect("writable");
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_an_entry_that_starts_as_a_property_file_but_cannot_be_decoded_is_skipped() {
    let dir = temp("truncated_entry");
    let game = game_with(&dir, &[shared(&[0, 1, 44], false)]);
    let hashes = game_hash_set(&game);
    let links: Vec<String> = (0..20_000)
        .map(|n| format!("DATA/Characters/Zed/Unique_{n}.bin"))
        .collect();
    let decoded = serialize_prop_links(&links, 3);
    let compressed = zstd::bulk::compress(&decoded, 3).expect("zstd");
    let stored = compressed[..compressed.len() / 2].to_vec();
    let mut writer = WadWriter::default();
    writer.insert(
        skin0_hash(),
        WriterEntry {
            kind: dekan_wad::CompressionType::Zstd as u8,
            subchunk_count: 0,
            first_subchunk: 0,
            uncompressed_size: decoded.len() as u64,
            checksum: dekan_wad::hash::content_checksum(&stored),
            payload: dekan_wad::writer::Payload::Memory(std::sync::Arc::from(stored)),
        },
    );
    let wad = dir.join("mod").join("Zed.wad.client");
    std::fs::create_dir_all(wad.parent().expect("parent")).expect("dir");
    std::fs::write(&wad, writer.to_bytes().expect("wad")).expect("wad");

    assert!(relink_wad(&wad, &game, &hashes).expect("repair").is_empty());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
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
fn test_a_prop_entry_that_does_not_parse_makes_the_mod_incompatible() {
    let dir = temp("corrupt_prop");
    let wad = dir.join("Zed.wad.client");
    write_wad(
        &wad,
        &[(
            wad_path_hash("data/x.bin"),
            b"PROP\x03\x00truncated".to_vec(),
        )],
    );
    let compat = check_wad(&wad, &HashSet::new()).expect("check");
    assert!(
        !compat.is_compatible(),
        "a damaged PROP is not loaded silently"
    );
    assert_eq!(compat.unreadable.len(), 1);
    assert_eq!(compat.entries_skipped, 0);
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
