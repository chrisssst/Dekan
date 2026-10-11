use super::tests::*;
use super::*;

#[test]
fn test_generated_bin_facts_report_what_changed() {
    let chroma = classified_skin_bin("Zed", 70, 2);
    let identity = SlotIdentity {
        classification: Some(1),
        parent: 0,
    };
    let out = retarget_skin_bin(&chroma, "Zed", 70, 0, Some(identity)).expect("retarget");
    let before = skin_bin_facts(&chroma).expect("source facts");
    let after = skin_bin_facts(&out).expect("generated facts");
    assert_eq!(before.classification, Some(2));
    assert_eq!(after.classification, Some(1));
    assert_eq!(after.links[0], "DATA/Characters/Zed/Skins/Skin70.bin");
    assert_eq!(after.objects, 1);
    assert_eq!(skin_bin_facts(b"not a bin"), None);
}

#[test]
fn test_relocating_a_bin_moves_keys_and_references_and_adds_a_link_once() {
    let bin = graph_bin("Zed", 5);
    let from = prop_key_hash("Characters/Zed/Animations/Skin5");
    let to = prop_key_hash("Characters/Zed/Animations/Skin0");
    let moves = std::collections::BTreeMap::from([(from, to)]);
    let out = relocate_prop(
        &bin,
        &moves,
        Some("DATA/Characters/Zed/Animations/Skin5.bin"),
    )
    .expect("relocate");
    let again = relocate_prop(
        &out,
        &moves,
        Some("data/characters/zed/animations/skin5.bin"),
    )
    .expect("again");
    let parsed = parse_prop_file(&again).expect("parse");
    assert_eq!(parsed.entries[0].key_hash, to);
    assert_eq!(skin_field(&again, "objectPath"), Some(to));
    assert_eq!(
        parsed.links.len(),
        1,
        "a link already there is not added twice"
    );
}

#[test]
fn test_the_graph_test_variant_moves_the_skins_own_graph_to_slot_0() {
    let game = standard_game(
        "graph_slot0",
        &[
            (
                wad_path_hash(&skin_bin("zed", 5)),
                skin_with_graph("Zed", 5, 1),
            ),
            (wad_path_hash(&animation_bin("zed", 5)), graph_bin("Zed", 5)),
        ],
    );
    let mods_dir = game.join("mods");
    let slot0 = prop_key_hash("Characters/Zed/Animations/Skin0");
    let wad_dir = |folder: &str| {
        mods_dir
            .join(folder)
            .join("WAD")
            .join("Zed.wad.client")
            .join("data")
            .join("characters")
            .join("zed")
    };

    let plain = StandardChampion::open(&game, "Zed")
        .expect("open")
        .build_mod(5, None, &mods_dir)
        .expect("build");
    assert!(
        !wad_dir(&plain).join("animations").exists(),
        "off by default"
    );
    let skin0 = std::fs::read(wad_dir(&plain).join("skins").join("skin0.bin")).expect("skin0");
    assert_eq!(
        graph_link_of(&skin0),
        Some(prop_key_hash("Characters/Zed/Animations/Skin5"))
    );

    let moved = StandardChampion::open(&game, "Zed")
        .expect("open")
        .with_options(GenerationOptions {
            graph_in_slot0: true,
            chroma_keeps_classification: false,
        })
        .build_mod(5, None, &mods_dir)
        .expect("build");
    let skin0 = std::fs::read(wad_dir(&moved).join("skins").join("skin0.bin")).expect("skin0");
    assert_eq!(graph_link_of(&skin0), Some(slot0));
    assert!(
        parse_prop_file(&skin0)
            .expect("parse")
            .links
            .contains(&"DATA/Characters/Zed/Animations/Skin0.bin".to_string())
    );
    let graph = std::fs::read(wad_dir(&moved).join("animations").join("skin0.bin")).expect("graph");
    let parsed = parse_prop_file(&graph).expect("parse graph");
    assert_eq!(parsed.entries[0].key_hash, slot0);
    assert_eq!(skin_field(&graph, "objectPath"), Some(slot0));
    assert_eq!(
        parsed.links,
        vec!["DATA/Characters/Zed/Animations/Skin5.bin".to_string()]
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_the_classification_test_variant_keeps_the_chromas_own() {
    let game = standard_game(
        "chroma_class",
        &[
            (
                wad_path_hash(&skin_bin("zed", 0)),
                skin_object_bin("Zed", 0, 1, None),
            ),
            (
                wad_path_hash(&skin_bin("zed", 70)),
                skin_object_bin("Zed", 70, 2, Some(69)),
            ),
        ],
    );
    let mods_dir = game.join("mods");
    let read = |champion: StandardChampion| {
        let folder = champion.build_mod(70, None, &mods_dir).expect("build");
        std::fs::read(
            mods_dir
                .join(folder)
                .join("WAD")
                .join("Zed.wad.client")
                .join("data")
                .join("characters")
                .join("zed")
                .join("skins")
                .join("skin0.bin"),
        )
        .expect("skin0")
    };
    let slot = read(StandardChampion::open(&game, "Zed").expect("open"));
    assert_eq!(classification_of(&slot), 1, "default: the slot's identity");
    assert_eq!(skin_field(&slot, "skinParent"), Some(0));
    let kept = read(
        StandardChampion::open(&game, "Zed")
            .expect("open")
            .with_options(GenerationOptions {
                graph_in_slot0: false,
                chroma_keeps_classification: true,
            }),
    );
    assert_eq!(classification_of(&kept), 2, "variant: the chroma's own");
    assert_eq!(
        skin_field(&kept, "skinParent"),
        Some(0),
        "the parent still follows the slot"
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_prewarm_lists_only_champion_archives() {
    let root = std::env::temp_dir().join(format!("dekan_prewarm_{}", std::process::id()));
    let champions = root.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("dir");
    for name in [
        "Garen.wad.client",
        "Garen.pt_BR.wad.client",
        "Viego.wad.client",
        "notes.txt",
        "..wad.client",
    ] {
        std::fs::write(champions.join(name), b"").expect("file");
    }
    assert_eq!(champion_aliases(&root), vec!["Garen", "Viego"]);
    std::fs::remove_dir_all(&root).expect("cleanup");
    assert!(
        champion_aliases(&root).is_empty(),
        "a missing folder lists nothing"
    );
}

#[test]
fn a_companion_cache_counts_only_for_the_exact_wad_it_was_made_from() {
    let root = std::env::temp_dir().join(format!("dekan_companion_cache_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet
    let champions = root
        .join("Game")
        .join("DATA")
        .join("FINAL")
        .join("Champions");
    std::fs::create_dir_all(&champions).expect("champions");
    let state = root.join("state");
    std::fs::create_dir_all(&state).expect("state");
    let wad = champions.join("Zed.wad.client");
    std::fs::write(&wad, b"not even a real wad").expect("wad");
    let game = root.join("Game");

    assert!(
        !companion_cache_is_current(&game, &state, "Zed"),
        "no cache yet"
    );
    let cache = CharacterCache {
        source: wad_stamp(&wad),
        characters: ["zedshadow".to_owned()].into(),
    };
    std::fs::write(
        state.join("companion_names_zed.json"),
        serde_json::to_vec(&cache).expect("json"),
    )
    .expect("cache");
    assert!(
        companion_cache_is_current(&game, &state, "Zed"),
        "valid without opening the WAD"
    );

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&wad, b"a patched wad of another size").expect("patch");
    assert!(
        !companion_cache_is_current(&game, &state, "Zed"),
        "a patched WAD invalidates the cache"
    );
    assert!(
        !companion_cache_is_current(&game, &state, "Ahri"),
        "another champion has no cache"
    );
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}
