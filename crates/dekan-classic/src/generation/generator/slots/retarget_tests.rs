use super::tests::*;
use super::*;

#[test]
fn test_retarget_keeps_only_the_skin_objects_rekeyed_and_links_the_original() {
    let source = skin_bin_fixture("Jade_Annie", 5);
    let out = retarget_skin_bin(&source, "Jade_Annie", 5, 301, None).expect("retarget");
    let parsed = parse_prop_file(&out).expect("parse output");

    assert_eq!(
        parsed.links,
        vec![
            "DATA/Characters/Jade_Annie/Skins/Skin5.bin".to_string(),
            "DATA/Characters/Annie/Annie.bin".to_string(),
        ],
        "the source bin comes first, then its own dependencies"
    );
    assert_eq!(
        parsed.entries.len(),
        2,
        "only the skin object and its Resources survive"
    );
    assert_eq!(
        parsed.entries[0].key_hash,
        prop_key_hash("Characters/Jade_Annie/Skins/Skin301")
    );
    assert_eq!(
        parsed.entries[1].key_hash,
        prop_key_hash("Characters/Jade_Annie/Skins/Skin301/Resources")
    );
    assert_eq!(
        parsed.entries[0].body,
        u32_body(0xAAAA_AAAA),
        "bodies are carried untouched"
    );
    assert_eq!(parsed.entries[0].class_hash, 1);
}

#[test]
fn test_the_target_slot_takes_the_identity_the_game_gives_that_slot() {
    let slot0 = slot_identity(&skin_object_bin("Zed", 0, 1, None)).expect("identity");
    assert_eq!(
        slot0,
        SlotIdentity {
            classification: Some(1),
            parent: 0
        },
        "a base skin has no parent"
    );
    let chroma = skin_object_bin("Zed", 70, 2, Some(69));
    let out = retarget_skin_bin(&chroma, "Zed", 70, 0, Some(slot0)).expect("retarget");
    assert_eq!(classification_of(&out), 1);
    assert_eq!(skin_field(&out, "skinParent"), Some(0));
    let untouched = retarget_skin_bin(&chroma, "Zed", 70, 301, None).expect("classic slot");
    assert_eq!(classification_of(&untouched), 2, "no identity, nothing set");
    assert_eq!(skin_field(&untouched, "skinParent"), Some(69));
}

#[test]
fn test_every_reference_to_a_moved_object_follows_it() {
    let source = skin_object_bin("Viego", 1, 1, None);
    let out = retarget_skin_bin(&source, "Viego", 1, 0, None).expect("retarget");
    assert_eq!(
        skin_field(&out, "objectPath"),
        Some(prop_key_hash("Characters/Viego/Skins/Skin0")),
        "the object names itself by its new key"
    );
    assert_eq!(
        skin_field(&out, "mResourceResolver"),
        Some(prop_key_hash("Characters/Viego/Skins/Skin0/Resources")),
        "the resolver link follows the re-keyed resolver"
    );
    let body = &parse_prop_file(&out).expect("parse").entries[0].body;
    assert!(
        !dekan_wad::prop::reference_values(body)
            .expect("references")
            .contains(&prop_key_hash("Characters/Viego/Skins/Skin1")),
        "nothing points at the source key any more"
    );
}

#[test]
fn test_retarget_refuses_a_bin_without_the_skin_object() {
    let source = skin_bin_fixture("Jade_Annie", 5);
    assert!(retarget_skin_bin(&source, "Jade_Annie", 6, 0, None).is_err());
}

#[test]
fn test_retarget_animation_bin_rekeys_and_links() {
    let source_prop = serialize_prop_file(&dekan_wad::prop::PropFile {
        version: 3,
        links: vec![],
        entries: vec![dekan_wad::prop::PropEntry {
            class_hash: 0x1234_5678,
            key_hash: prop_key_hash("Characters/Zed/Animations/Skin15"),
            body: b"anim_graph_data".to_vec(),
        }],
    })
    .expect("serialize");

    let retargeted = retarget_animation_bin(&source_prop, "Zed", 15, 0).expect("retarget");
    let parsed = parse_prop_file(&retargeted).expect("parse");
    assert_eq!(
        parsed.entries[0].key_hash,
        prop_key_hash("Characters/Zed/Animations/Skin0")
    );
    assert_eq!(
        parsed.links,
        vec!["DATA/Characters/Zed/Animations/Skin15.bin"]
    );
}

#[test]
fn test_classic_champion_retargets_animation_bin() {
    let game = std::env::temp_dir().join(format!("dekan_classic_anim_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture may not exist yet
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");

    let anim_fixture = serialize_prop_file(&dekan_wad::prop::PropFile {
        version: 3,
        links: vec![],
        entries: vec![dekan_wad::prop::PropEntry {
            class_hash: 0x1234_5678,
            key_hash: prop_key_hash("Characters/Jade_Annie/Animations/Skin15"),
            body: b"anim_data".to_vec(),
        }],
    })
    .expect("serialize anim fixture");

    let wad = raw_wad(&[
        (
            wad_path_hash(&character_bin("jade_annie")),
            b"PROP".to_vec(),
        ),
        (
            wad_path_hash(&skin_bin("jade_annie", 15)),
            skin_bin_fixture("Jade_Annie", 15),
        ),
        (
            wad_path_hash(&animation_bin("jade_annie", 15)),
            anim_fixture,
        ),
    ]);
    std::fs::write(champions.join("Annie.wad.client"), wad).expect("write wad");
    let mods_dir = game.join("mods");

    let champion = ClassicChampion::open(&game, "Annie").expect("open");
    assert!(champion.has_skin("jade_annie", 15));

    let mut known = BTreeSet::new();
    known.insert("jade_annie".to_string());
    let folder = champion
        .build_mod(15, &[0], &known, &mods_dir)
        .expect("build classic mod");

    let anim_file = mods_dir
        .join(&folder)
        .join("WAD")
        .join("Annie.wad.client")
        .join("data")
        .join("characters")
        .join("jade_annie")
        .join("animations")
        .join("skin0.bin");
    assert!(
        anim_file.is_file(),
        "classic animation skin0.bin must be generated"
    );

    let bytes = std::fs::read(&anim_file).expect("read generated anim");
    let prop = parse_prop_file(&bytes).expect("parse generated anim prop");
    assert_eq!(
        prop.entries[0].key_hash,
        prop_key_hash("Characters/Jade_Annie/Animations/Skin0")
    );

    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}
