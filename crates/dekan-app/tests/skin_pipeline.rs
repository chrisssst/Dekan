use std::collections::{BTreeSet, HashMap};

use dekan_inject::{mod_compat, overlay_builder};
use dekan_wad::hash::{prop_key_hash, wad_path_hash};
use dekan_wad::prop::{PropEntry, parse_prop_file};
use dekan_wad::wad::WadFile;

mod support;

use support::*;

#[tokio::test]
async fn a_generated_chroma_loads_completely_in_what_the_game_would_mount() {
    let root = Scratch::new("chroma");
    let game = game_install(&root.0);
    let prepared = prepare(&root.0, &game, None).await;
    let overlay = root.0.join("Dekan").join("overlay");
    build(&game, &prepared, &overlay);
    let mounted = Mounted::new(&game.dir, &overlay);

    assert_eq!(
        mounted.served,
        BTreeSet::from([CHAMPIONS.to_owned(), MAP11.to_owned()]),
        "the champion WAD and the map that shares the shadow are served; the TFT map never is"
    );

    let skin0 = mounted
        .read(CHAMPIONS, "data/characters/zed/skins/skin0.bin")
        .expect("zed skin0.bin");
    let (key, classification, graph) = skin_fields(&skin0);
    assert_eq!(
        key,
        prop_key_hash("Characters/Zed/Skins/Skin0"),
        "the chroma sits in slot 0"
    );
    assert_eq!(
        classification,
        Some(1),
        "slot 0 is classified as a base skin"
    );
    assert_eq!(
        graph,
        prop_key_hash("Characters/Zed/Animations/Skin69"),
        "the chroma keeps its own animation graph"
    );

    let game_skin70 = WadFile::open(&game.dir.join(CHAMPIONS))
        .expect("game")
        .read(wad_path_hash("data/characters/zed/skins/skin70.bin"))
        .expect("read")
        .expect("skin70");
    let source = parse_prop_file(&game_skin70).expect("skin70");
    let generated = parse_prop_file(&skin0).expect("skin0");
    assert_eq!(generated.links[0], "DATA/Characters/Zed/Skins/Skin70.bin");
    assert_eq!(
        &generated.links[1..],
        &source.links[..],
        "every dependency of the source is kept"
    );
    for (made, original) in generated.entries.iter().zip(&source.entries) {
        let mut expected = original.body.clone();
        if made.class_hash == SKIN_DATA {
            expected[7..11].copy_from_slice(&1u32.to_le_bytes());
        }
        assert_eq!(
            made.body, expected,
            "object {:#x} is the game's bytes",
            made.class_hash
        );
    }

    let reachable = reachable_objects(&mounted, "data/characters/zed/skins/skin0.bin")
        .unwrap_or_else(|e| panic!("asset resolution failed: {e}"));
    assert!(
        reachable.contains(&graph),
        "the animation graph the skin names is defined in a mounted, linked bin"
    );

    let shadow = mounted.find("data/characters/zedshadow/skins/skin0.bin");
    assert_eq!(
        shadow.len(),
        2,
        "the shadow is in the champion WAD and in Map11"
    );
    assert_eq!(
        shadow[0], shadow[1],
        "both mounted WADs agree on the shadow"
    );
    assert_ne!(
        shadow[0], game.shadow_skin0,
        "the shadow takes the skin's look"
    );
    let (shadow_key, _, shadow_graph) = skin_fields(&shadow[0]);
    assert_eq!(
        shadow_key,
        prop_key_hash("Characters/ZedShadow/Skins/Skin0")
    );
    assert_eq!(
        shadow_graph,
        prop_key_hash("Characters/ZedShadow/Animations/Skin69")
    );
    reachable_objects(&mounted, "data/characters/zedshadow/skins/skin0.bin")
        .unwrap_or_else(|e| panic!("shadow asset resolution failed: {e}"));

    for (wad, path) in [
        (CHAMPIONS, "data/characters/zed/animations/skin0.bin"),
        (CHAMPIONS, "data/characters/zedshadow/animations/skin0.bin"),
        (MAP11, "data/characters/zedshadow/animations/skin0.bin"),
    ] {
        let original = WadFile::open(&game.dir.join(wad))
            .expect("game")
            .read(wad_path_hash(path))
            .expect("read");
        assert_eq!(
            mounted.read(wad, path),
            original,
            "{path} in {wad}: the base graph is untouched"
        );
    }
    assert_eq!(
        mounted.read(MAP22, "data/characters/zedshadow/skins/skin0.bin"),
        Some(game.shadow_skin0.clone())
    );
}

#[tokio::test]
async fn rebuilt_wads_keep_the_games_header_and_every_untouched_byte() {
    let root = Scratch::new("faithful");
    let game = game_install(&root.0);
    let prepared = prepare(&root.0, &game, None).await;
    let overlay = root.0.join("Dekan").join("overlay");
    build(&game, &prepared, &overlay);

    for relative in [CHAMPIONS, MAP11] {
        let original = std::fs::read(game.dir.join(relative)).expect("game wad");
        let rebuilt = std::fs::read(overlay.join(relative)).expect("overlay wad");
        assert_eq!(
            &rebuilt[..268],
            &original[..268],
            "{relative}: signature and checksum are the game's"
        );
        let game_wad = WadFile::open(&game.dir.join(relative)).expect("game");
        let built = WadFile::open(&overlay.join(relative)).expect("built");
        let changed = [
            wad_path_hash("data/characters/zed/skins/skin0.bin"),
            wad_path_hash("data/characters/zedshadow/skins/skin0.bin"),
        ];
        for entry in game_wad.toc() {
            let rebuilt_entry = built.entry(entry.path_hash).expect("entry kept");
            if changed.contains(&entry.path_hash) {
                continue;
            }
            assert_eq!(
                built.read_raw(rebuilt_entry).expect("raw"),
                game_wad.read_raw(entry).expect("raw"),
                "{relative}: {:#x} is copied byte for byte",
                entry.path_hash
            );
        }
    }
}

#[tokio::test]
async fn an_imported_custom_skin_mod_is_applied_after_the_generated_skin() {
    let root = Scratch::new("custom");
    let game = game_install(&root.0);
    let custom = [
        (TEXTURE, b"custom glowing texture".to_vec()),
        (NEW_TEXTURE, b"brand new glow layer".to_vec()),
    ];
    let prepared = prepare(&root.0, &game, Some(&custom)).await;
    assert_eq!(
        prepared.mods.len(),
        2,
        "the generated skin and the staged custom mod"
    );

    let game_hashes =
        mod_compat::game_hash_set(&overlay_builder::get_or_index_game(&game.dir).expect("index"));
    for name in &prepared.mods[1..] {
        let wad = prepared
            .mods_dir
            .join(name)
            .join("WAD")
            .join("Zed.wad.client");
        let compat = mod_compat::check(&wad, &game_hashes, &std::collections::HashSet::new())
            .expect("compat");
        assert!(compat.is_compatible(), "{name}: {:?}", compat.dangling);
    }

    let overlay = root.0.join("Dekan").join("overlay");
    build(&game, &prepared, &overlay);
    let mounted = Mounted::new(&game.dir, &overlay);
    assert_eq!(
        mounted.read(CHAMPIONS, TEXTURE).as_deref(),
        Some(&b"custom glowing texture"[..])
    );
    assert_eq!(
        mounted.read(CHAMPIONS, NEW_TEXTURE).as_deref(),
        Some(&b"brand new glow layer"[..])
    );
    let (key, classification, _) = skin_fields(
        &mounted
            .read(CHAMPIONS, "data/characters/zed/skins/skin0.bin")
            .expect("skin0"),
    );
    assert_eq!(
        (key, classification),
        (prop_key_hash("Characters/Zed/Skins/Skin0"), Some(1))
    );
    let shadow = mounted.find("data/characters/zedshadow/skins/skin0.bin");
    assert_eq!(
        shadow[0], shadow[1],
        "adding a custom mod keeps the map consistent"
    );
}

#[tokio::test]
async fn a_custom_mod_linking_a_missing_bin_is_reported_as_dangling() {
    let root = Scratch::new("dangling");
    let game = game_install(&root.0);
    let broken = prop(
        &["DATA/Characters/Zed/Skins/Skin999.bin".to_owned()],
        vec![PropEntry {
            class_hash: SKIN_DATA,
            key_hash: prop_key_hash("Characters/Zed/Skins/Skin0"),
            body: 0u16.to_le_bytes().to_vec(),
        }],
    );
    let prepared = prepare(
        &root.0,
        &game,
        Some(&[("data/characters/zed/skins/skin0.bin", broken)]),
    )
    .await;
    let game_hashes =
        mod_compat::game_hash_set(&overlay_builder::get_or_index_game(&game.dir).expect("index"));
    let wad = prepared
        .mods_dir
        .join(&prepared.mods[1])
        .join("WAD")
        .join("Zed.wad.client");
    let compat =
        mod_compat::check(&wad, &game_hashes, &std::collections::HashSet::new()).expect("compat");
    assert!(
        !compat.is_compatible(),
        "a link to a bin the patch lacks must be caught before the game"
    );
}

#[tokio::test]
async fn the_overlay_is_deterministic_and_an_identical_rebuild_writes_nothing() {
    let first_root = Scratch::new("determinism_a");
    let second_root = Scratch::new("determinism_b");
    let mut outputs: Vec<HashMap<&str, Vec<u8>>> = Vec::new();
    for root in [&first_root, &second_root] {
        let game = game_install(&root.0);
        let prepared = prepare(&root.0, &game, None).await;
        let overlay = root.0.join("Dekan").join("overlay");
        build(&game, &prepared, &overlay);
        let again = build(&game, &prepared, &overlay);
        assert_eq!(again.written, 0, "an identical rebuild rewrites nothing");
        outputs.push(
            [CHAMPIONS, MAP11]
                .into_iter()
                .map(|relative| {
                    (
                        relative,
                        std::fs::read(overlay.join(relative)).expect("overlay"),
                    )
                })
                .collect(),
        );
    }
    assert_eq!(
        outputs[0], outputs[1],
        "the same fixture always gives the same bytes"
    );
}
