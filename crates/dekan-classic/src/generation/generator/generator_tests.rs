use super::*;

fn u32_body(value: u32) -> Vec<u8> {
    let mut body = 1u16.to_le_bytes().to_vec();
    body.extend_from_slice(&0x1234_5678u32.to_le_bytes());
    body.push(7);
    body.extend_from_slice(&value.to_le_bytes());
    body
}

fn skin_bin_fixture(character: &str, skin: u32) -> Vec<u8> {
    let prefix = format!("Characters/{character}/Skins/Skin{skin}");
    serialize_prop_file(&PropFile {
        version: 3,
        links: vec!["DATA/Characters/Annie/Annie.bin".into()],
        entries: vec![
            PropEntry {
                class_hash: 1,
                key_hash: prop_key_hash(&prefix),
                body: u32_body(0xAAAA_AAAA),
            },
            PropEntry {
                class_hash: 2,
                key_hash: prop_key_hash(&format!("{prefix}/Resources")),
                body: u32_body(0xBBBB_BBBB),
            },
            PropEntry {
                class_hash: 3,
                key_hash: prop_key_hash("Characters/Annie/Skins/Skin5/Particles/Fire"),
                body: u32_body(0xCCCC_CCCC),
            },
        ],
    })
    .expect("fixture")
}

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

fn skin_object_body(
    character: &str,
    skin: u32,
    classification: u32,
    parent: Option<i32>,
) -> Vec<u8> {
    let prefix = format!("Characters/{character}/Skins/Skin{skin}");
    let mut fields: Vec<(u32, u8, Vec<u8>)> = vec![
        (
            prop_key_hash("skinClassification"),
            7,
            classification.to_le_bytes().to_vec(),
        ),
        (
            prop_key_hash("objectPath"),
            17,
            prop_key_hash(&prefix).to_le_bytes().to_vec(),
        ),
        (
            prop_key_hash("mResourceResolver"),
            0x84,
            prop_key_hash(&format!("{prefix}/Resources"))
                .to_le_bytes()
                .to_vec(),
        ),
    ];
    if let Some(parent) = parent {
        fields.push((
            prop_key_hash("skinParent"),
            6,
            parent.to_le_bytes().to_vec(),
        ));
    }
    let mut body = (fields.len() as u16).to_le_bytes().to_vec();
    for (name, kind, value) in fields {
        body.extend_from_slice(&name.to_le_bytes());
        body.push(kind);
        body.extend_from_slice(&value);
    }
    body
}

fn skin_object_bin(
    character: &str,
    skin: u32,
    classification: u32,
    parent: Option<i32>,
) -> Vec<u8> {
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: 0x9b67_e9f6,
            key_hash: prop_key_hash(&format!("Characters/{character}/Skins/Skin{skin}")),
            body: skin_object_body(character, skin, classification, parent),
        }],
    })
    .expect("fixture")
}

fn classified_skin_bin(character: &str, skin: u32, classification: u32) -> Vec<u8> {
    skin_object_bin(character, skin, classification, None)
}

fn skin_field(bin: &[u8], name: &str) -> Option<u32> {
    let body = &parse_prop_file(bin).expect("parse").entries[0].body;
    field_value(body, &[prop_key_hash(name)])
        .expect("walk")
        .and_then(|v| v.as_u32())
}

fn classification_of(bin: &[u8]) -> u32 {
    skin_field(bin, "skinClassification").expect("classification")
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
fn test_skin_numbers_and_slots_follow_rose() {
    assert_eq!(skin_number(60_012_301), 301);
    assert_eq!(skin_number(12_005), 5, "a chroma has its own skin number");
    assert_eq!(slots_for(None), vec![0, 301, 302]);
    assert_eq!(slots_for(Some(60_012_007)), vec![0, 301, 302, 7]);
    assert_eq!(
        slots_for(Some(12_301)),
        vec![0, 301, 302],
        "no duplicate slot"
    );
}

#[test]
fn test_aliases_are_restricted_to_safe_names() {
    assert!(is_safe_alias("MonkeyKing"));
    assert!(is_safe_alias("Jade_X1"));
    assert!(!is_safe_alias(""));
    assert!(!is_safe_alias("../Annie"));
    assert!(!is_safe_alias("Annie.wad"));
}

#[test]
fn test_jade_characters_are_read_from_the_hash_table_and_cached() {
    let dir = std::env::temp_dir().join(format!("dekan_jade_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("dir");
    let table = dir.join("hashes.game.txt");
    std::fs::write(
        &table,
        "0123 data/characters/jade_annie/jade_annie.bin\n\
             4567 data/characters/jade_annietibbers/skins/skin0.bin\n\
             89ab data/characters/annie/annie.bin\n\
             cdef data/characters/jade_bad name/x.bin\n",
    )
    .expect("table");
    let cache = dir.join("cache.json");

    let found = jade_characters(&table, &cache);
    let expected: BTreeSet<String> = ["jade_annie", "jade_annietibbers"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(found, expected);
    assert!(cache.is_file());
    assert_eq!(
        jade_characters(&table, &cache),
        expected,
        "served from the cache"
    );
    assert!(
        jade_characters(&dir.join("missing.txt"), &cache).is_empty(),
        "no table degrades to no companions"
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

fn raw_wad(entries: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut wad = vec![0u8; 272 + 32 * entries.len()];
    wad[0..4].copy_from_slice(b"RW\x03\x04");
    wad[268..272].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    for (i, (hash, payload)) in entries.iter().enumerate() {
        let offset = wad.len() as u32;
        let toc = 272 + 32 * i;
        wad[toc..toc + 8].copy_from_slice(&hash.to_le_bytes());
        wad[toc + 8..toc + 12].copy_from_slice(&offset.to_le_bytes());
        wad[toc + 12..toc + 16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        wad[toc + 16..toc + 20].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        wad.extend_from_slice(payload);
    }
    wad
}

#[test]
fn test_companions_are_recovered_from_bins_without_a_hash_table() {
    let game = std::env::temp_dir().join(format!("dekan_jade_bins_{}", std::process::id()));
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");

    let mut bin = b"PROP".to_vec();
    bin.extend_from_slice(b"...Characters/Jade_Annie/Skins/Skin0...");
    bin.extend_from_slice(b"...Characters/Jade_AnnieTibbers/Skins/Skin0...");

    bin.extend_from_slice(b"...Characters/Jade_Soraka/Skins/Skin0...");

    let texture = b"DDS characters/jade_fake/x".to_vec();
    let wad = raw_wad(&[
        (1, bin),
        (2, texture),
        (
            wad_path_hash(&character_bin("jade_annie")),
            b"PROP".to_vec(),
        ),
        (
            wad_path_hash(&character_bin("jade_annietibbers")),
            b"PROP".to_vec(),
        ),
    ]);
    std::fs::write(champions.join("Annie.wad.client"), wad).expect("write wad");

    let champion = ClassicChampion::open(&game, "Annie").expect("open");
    let names = champion.jade_names_in_bins();
    assert!(names.contains("jade_annietibbers"));
    assert!(!names.contains("jade_fake"), "{names:?}");
    assert_eq!(
        champion.present_characters(&names),
        vec!["jade_annie".to_owned(), "jade_annietibbers".to_owned()]
    );

    let cached = champion.jade_names_from_bins_cached(&game);
    assert_eq!(cached, names);
    assert!(game.join("classic_bin_names_annie.json").is_file());
    assert_eq!(champion.jade_names_from_bins_cached(&game), names);

    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_standard_champion_generates_slot_0_redirection() {
    let game = std::env::temp_dir().join(format!("dekan_std_wad_{}", std::process::id()));
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");

    let skin_bin_content = skin_bin_fixture("Zed", 1);
    let wad = raw_wad(&[(wad_path_hash(&skin_bin("zed", 1)), skin_bin_content)]);
    std::fs::write(champions.join("Zed.wad.client"), wad).expect("write wad");

    let mods_dir = game.join("mods");
    std::fs::create_dir_all(&mods_dir).expect("mods dir");

    let champion = StandardChampion::open(&game, "Zed").expect("open");
    assert!(champion.has_skin(1));
    assert!(!champion.has_skin(2));

    let folder = champion.build_mod(1, None, &mods_dir).expect("build mod");
    assert_eq!(folder, "std_zed_1");
    assert!(
        mods_dir
            .join(&folder)
            .join("WAD")
            .join("Zed.wad.client")
            .join("data")
            .join("characters")
            .join("zed")
            .join("skins")
            .join("skin0.bin")
            .is_file()
    );
    assert!(
        mods_dir
            .join(&folder)
            .join("META")
            .join("info.json")
            .is_file()
    );

    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

fn tibbers_bin(skin: u32) -> Vec<u8> {
    let mut prop = parse_prop_file(&skin_bin_fixture("annietibbers", skin)).expect("fixture");
    prop.links
        .push("DATA/Characters/AnnieTibbers/AnnieTibbers.bin".into());
    serialize_prop_file(&prop).expect("fixture")
}

#[test]
fn test_standard_champion_retargets_the_companion_too() {
    let game = std::env::temp_dir().join(format!("dekan_std_pet_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture may not exist yet
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");
    let wad = raw_wad(&[
        (
            wad_path_hash(&skin_bin("annie", 5)),
            skin_bin_fixture("Annie", 5),
        ),
        (wad_path_hash(&skin_bin("annietibbers", 5)), tibbers_bin(5)),
    ]);
    std::fs::write(champions.join("Annie.wad.client"), wad).expect("write wad");
    let mods_dir = game.join("mods");

    let folder = StandardChampion::open(&game, "Annie")
        .expect("open")
        .build_mod(5, None, &mods_dir)
        .expect("build");
    let chars = mods_dir
        .join(&folder)
        .join("WAD")
        .join("Annie.wad.client")
        .join("data")
        .join("characters");
    for (dir, character) in [("annie", "Annie"), ("annietibbers", "annietibbers")] {
        let bytes = std::fs::read(chars.join(dir).join("skins").join("skin0.bin"))
            .unwrap_or_else(|e| panic!("{dir} skin0.bin: {e}"));
        let prop = parse_prop_file(&bytes).expect("valid PROP");
        let keys: Vec<u32> = prop.entries.iter().map(|e| e.key_hash).collect();
        assert!(
            keys.contains(&prop_key_hash(&format!(
                "Characters/{character}/Skins/Skin0"
            ))),
            "{dir}: the skin object is re-keyed to slot 0"
        );
        assert!(
            !keys.contains(&prop_key_hash(&format!(
                "Characters/{character}/Skins/Skin5"
            ))),
            "{dir}: nothing left under the source slot"
        );
        assert_eq!(
            prop.links.first(),
            Some(&format!("DATA/Characters/{character}/Skins/Skin5.bin")),
            "{dir}: links the original skin bin for everything else"
        );
        assert!(
            prop.links
                .contains(&"DATA/Characters/Annie/Annie.bin".to_string()),
            "{dir}: keeps the source bin's own dependencies"
        );
    }
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_broken_companion_bin_still_yields_the_champion_skin() {
    let game = std::env::temp_dir().join(format!("dekan_std_badpet_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture may not exist yet
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");
    let wad = raw_wad(&[
        (
            wad_path_hash(&skin_bin("annie", 5)),
            skin_bin_fixture("Annie", 5),
        ),
        (
            wad_path_hash(&skin_bin("annietibbers", 5)),
            b"not a prop file".to_vec(),
        ),
    ]);
    std::fs::write(champions.join("Annie.wad.client"), wad).expect("write wad");
    let mods_dir = game.join("mods");
    let folder = StandardChampion::open(&game, "Annie")
        .expect("open")
        .build_mod(5, None, &mods_dir)
        .expect("the champion skin is still built");
    let chars = mods_dir
        .join(&folder)
        .join("WAD")
        .join("Annie.wad.client")
        .join("data")
        .join("characters");
    assert!(
        chars
            .join("annie")
            .join("skins")
            .join("skin0.bin")
            .is_file()
    );
    assert!(
        !chars.join("annietibbers").exists(),
        "no half-written companion"
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_standard_champion_live_wad_if_installed() {
    let game = Path::new(r"D:\Riot Games\League of Legends\Game");
    if !game
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join("Zed.wad.client")
        .is_file()
    {
        return;
    }
    let champion = StandardChampion::open(game, "Zed").expect("open live zed wad");
    let zed_skins = champion.skin_numbers(50);
    assert!(zed_skins.contains(&1), "Zed must have skin 1");
    assert!(zed_skins.contains(&4), "Zed chroma 4 must have skin4.bin");
    let mods_dir = std::env::temp_dir().join(format!("dekan_live_mod_{}", std::process::id()));
    let folder = champion
        .build_mod(1, None, &mods_dir)
        .expect("build live mod");
    assert_eq!(folder, "std_zed_1");
    assert!(
        mods_dir
            .join(&folder)
            .join("WAD")
            .join("Zed.wad.client")
            .join("data")
            .join("characters")
            .join("zed")
            .join("skins")
            .join("skin0.bin")
            .is_file()
    );
    let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup

    let annie = StandardChampion::open(game, "Annie").expect("open live annie wad");
    assert!(annie.has_skin(1), "Annie must have skin 1 in live patch");
    let folder = annie
        .build_mod(1, None, &mods_dir)
        .expect("build live annie mod");
    assert_eq!(folder, "std_annie_1");
    let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup

    if let Ok(garen) = StandardChampion::open(game, "Garen") {
        if garen.has_skin(44) {
            let folder = garen
                .build_mod(44, None, &mods_dir)
                .expect("build live garen 44 mod");
            assert_eq!(folder, "std_garen_44");

            let skin_file = mods_dir
                .join(&folder)
                .join("WAD")
                .join("Garen.wad.client")
                .join("data")
                .join("characters")
                .join("garen")
                .join("skins")
                .join("skin0.bin");
            assert!(skin_file.is_file(), "skin0.bin must be generated");

            let skin_bytes = std::fs::read(&skin_file).expect("read generated skin0.bin");
            let skin_prop = parse_prop_file(&skin_bytes).expect("parse generated skin0.bin");
            assert_eq!(
                skin_prop.entries[0].key_hash,
                prop_key_hash("Characters/Garen/Skins/Skin0")
            );
            assert_eq!(
                skin_prop.links.first().map(String::as_str),
                Some("DATA/Characters/Garen/Skins/Skin44.bin"),
                "skin must link to source skin 44 bin first"
            );
            let source = garen
                .wad
                .read(wad_path_hash(&skin_bin("garen", 44)))
                .expect("read")
                .expect("skin44.bin");
            let source_links = parse_prop_file(&source).expect("parse skin44.bin").links;
            for link in &source_links {
                assert!(
                    skin_prop.links.contains(link),
                    "skin0.bin must keep skin44.bin's dependency {link}"
                );
            }

            assert!(
                skin_prop
                    .links
                    .contains(&"DATA/Characters/Garen/Animations/Skin44.bin".to_string()),
                "the skin's own animation graph is reached through its links"
            );
            assert!(
                !mods_dir
                    .join(&folder)
                    .join("WAD")
                    .join("Garen.wad.client")
                    .join("data")
                    .join("characters")
                    .join("garen")
                    .join("animations")
                    .join("skin0.bin")
                    .exists(),
                "the base animation graph is never replaced"
            );

            let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup
        }
    }

    if zed_skins.contains(&15) {
        let folder = champion
            .build_mod(15, None, &mods_dir)
            .expect("build live legendary mod");
        assert_eq!(folder, "std_zed_15");
        let characters = mods_dir
            .join(&folder)
            .join("WAD")
            .join("Zed.wad.client")
            .join("data")
            .join("characters")
            .join("zed");
        assert!(
            !characters.join("animations").join("skin0.bin").exists(),
            "the base animation graph is never replaced"
        );
        let skin = std::fs::read(characters.join("skins").join("skin0.bin")).expect("skin0");
        assert!(
            parse_prop_file(&skin)
                .expect("parse")
                .links
                .contains(&"DATA/Characters/Zed/Animations/Skin15.bin".to_string()),
            "the legendary graph is reached through the skin's links"
        );
        let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup
    }
}

#[test]
fn test_ultimate_skins_compatibility_if_installed() {
    let game = Path::new(r"D:\Riot Games\League of Legends\Game");
    if !game.join("DATA").join("FINAL").join("Champions").is_dir() {
        return;
    }

    let ultimates = [
        ("Lux", 7, "Elementalist Lux"),
        ("Sona", 6, "DJ Sona"),
        ("Udyr", 3, "Spirit Guard Udyr"),
        ("Ezreal", 5, "Pulsefire Ezreal"),
        ("MissFortune", 16, "Gun Goddess Miss Fortune"),
        ("Samira", 10, "Soul Fighter Samira"),
    ];

    let mods_dir = std::env::temp_dir().join(format!("dekan_ultimate_{}", std::process::id()));

    for (champ, skin, name) in ultimates {
        if let Ok(c) = StandardChampion::open(game, champ) {
            if c.has_skin(skin) {
                eprintln!("[ULTIMATE] {name} ({champ}, skin {skin})");

                let folder = c
                    .build_mod(skin, None, &mods_dir)
                    .expect("build ultimate mod");
                let skin_file = mods_dir
                    .join(&folder)
                    .join("WAD")
                    .join(format!("{champ}.wad.client"))
                    .join("data")
                    .join("characters")
                    .join(champ.to_ascii_lowercase())
                    .join("skins")
                    .join("skin0.bin");
                assert!(
                    skin_file.is_file(),
                    "ultimate skin0.bin must be generated for {name}"
                );

                let links = parse_prop_file(&std::fs::read(&skin_file).expect("skin0"))
                    .expect("parse skin0")
                    .links;
                let source = c
                    .read_skin_bin(&champ.to_ascii_lowercase(), skin)
                    .expect("read")
                    .expect("source bin");
                for link in parse_prop_file(&source).expect("parse source").links {
                    assert!(
                        links.contains(&link),
                        "{name} keeps the source dependency {link}, where its animation graph lives"
                    );
                }
                eprintln!("  -> Successfully built and verified {}!", folder);
                let _ = std::fs::remove_dir_all(&mods_dir); // ignore-ok: fixture cleanup
            } else {
                eprintln!(
                    "[ULTIMATE] {} ({}, skin {}) - not in wad",
                    name, champ, skin
                );
            }
        }
    }
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

fn companion_bin(character: &str, skin: u32) -> Vec<u8> {
    let prefix = format!("Characters/{character}/Skins/Skin{skin}");
    serialize_prop_file(&PropFile {
        version: 3,
        links: vec![format!("DATA/Characters/{character}/{character}.bin")],
        entries: vec![PropEntry {
            class_hash: 1,
            key_hash: prop_key_hash(&prefix),
            body: u32_body(0xAAAA_AAAA),
        }],
    })
    .expect("fixture")
}

fn standard_game(name: &str, entries: &[(u64, Vec<u8>)]) -> PathBuf {
    let game = std::env::temp_dir().join(format!("dekan_std_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture may not exist yet
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");
    std::fs::write(champions.join("Zed.wad.client"), raw_wad(entries)).expect("write wad");
    game
}

fn generated_skin0(mods_dir: &Path, folder: &str, character: &str) -> Option<PropFile> {
    let path = mods_dir
        .join(folder)
        .join("WAD")
        .join("Zed.wad.client")
        .join("data")
        .join("characters")
        .join(character)
        .join("skins")
        .join("skin0.bin");
    std::fs::read(path)
        .ok()
        .map(|bytes| parse_prop_file(&bytes).expect("valid PROP"))
}

#[test]
fn test_a_companion_outside_the_registry_is_found_in_the_bins_and_retargeted() {
    let game = standard_game(
        "shadow",
        &[
            (
                wad_path_hash(&skin_bin("zed", 10)),
                companion_bin("Zed", 10),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 10)),
                companion_bin("ZedShadow", 10),
            ),
            (
                wad_path_hash(&skin_bin("jade_zed", 10)),
                companion_bin("Jade_Zed", 10),
            ),
        ],
    );
    let mods_dir = game.join("mods");
    let champion = StandardChampion::open(&game, "Zed")
        .expect("open")
        .with_cache_dir(&game);
    assert!(champion.companions().contains("zedshadow"));
    assert!(
        !champion.companions().iter().any(|c| c.starts_with("jade_")),
        "Rift Classic characters belong to the Classic generator"
    );

    let folder = champion.build_mod(10, None, &mods_dir).expect("build");
    let shadow = generated_skin0(&mods_dir, &folder, "zedshadow").expect("shadow skin0.bin");
    assert_eq!(
        shadow.entries[0].key_hash,
        prop_key_hash("Characters/zedshadow/Skins/Skin0")
    );
    assert!(generated_skin0(&mods_dir, &folder, "jade_zed").is_none());
    assert!(game.join("companion_names_zed.json").is_file());
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_chroma_without_its_own_companion_bin_uses_the_base_skin_one() {
    let game = standard_game(
        "chroma",
        &[
            (
                wad_path_hash(&skin_bin("zed", 12)),
                companion_bin("Zed", 12),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 10)),
                companion_bin("ZedShadow", 10),
            ),
        ],
    );
    let mods_dir = game.join("mods");
    let champion = StandardChampion::open(&game, "Zed").expect("open");

    let without_base = champion.build_mod(12, None, &mods_dir).expect("build");
    assert!(generated_skin0(&mods_dir, &without_base, "zedshadow").is_none());

    let with_base = champion.build_mod(12, Some(10), &mods_dir).expect("build");
    let shadow = generated_skin0(&mods_dir, &with_base, "zedshadow").expect("shadow skin0.bin");
    assert_eq!(
        shadow.links.first().map(String::as_str),
        Some("DATA/Characters/zedshadow/Skins/Skin10.bin")
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_a_chroma_finds_its_parent_in_the_game_data_without_the_client() {
    let game = standard_game(
        "chroma_parent",
        &[
            (
                wad_path_hash(&skin_bin("zed", 12)),
                skin_object_bin("Zed", 12, 2, Some(10)),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 10)),
                companion_bin("ZedShadow", 10),
            ),
        ],
    );
    let mods_dir = game.join("mods");
    let champion = StandardChampion::open(&game, "Zed").expect("open");
    assert_eq!(champion.parent_skin(12), Some(10));
    assert_eq!(champion.parent_skin(10), None, "no bin, no parent");

    let folder = champion.build_mod(12, None, &mods_dir).expect("build");
    let shadow = generated_skin0(&mods_dir, &folder, "zedshadow").expect("shadow skin0.bin");
    assert_eq!(
        shadow.links.first().map(String::as_str),
        Some("DATA/Characters/zedshadow/Skins/Skin10.bin"),
        "the companion comes from the parent skin the game names"
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_the_client_names_the_classic_character_when_the_alias_does_not() {
    let game = std::env::temp_dir().join(format!("dekan_classic_wukong_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture may not exist yet
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");
    let wad = raw_wad(&[
        (
            wad_path_hash(&character_bin("jade_wukong")),
            b"PROP".to_vec(),
        ),
        (
            wad_path_hash(&skin_bin("jade_wukong", 3)),
            skin_bin_fixture("Jade_Wukong", 3),
        ),
    ]);
    std::fs::write(champions.join("MonkeyKing.wad.client"), wad).expect("write wad");
    let mods_dir = game.join("mods");
    let known = BTreeSet::new();

    let derived = ClassicChampion::open(&game, "MonkeyKing").expect("open");
    assert_eq!(derived.main_character(), "jade_monkeyking");
    assert!(
        derived
            .skin_numbers(derived.main_character(), 50)
            .is_empty()
    );
    assert!(
        derived.build_mod(3, &[0], &known, &mods_dir).is_err(),
        "the name derived from the archive does not exist in the game"
    );

    let named = ClassicChampion::open(&game, "MonkeyKing")
        .expect("open")
        .with_client_character(Some("Jade_Wukong"));
    assert_eq!(named.main_character(), "jade_wukong");
    assert_eq!(named.skin_numbers(named.main_character(), 50), vec![3]);
    let folder = named
        .build_mod(3, &[0], &known, &mods_dir)
        .expect("builds under the client's name");
    assert!(
        mods_dir
            .join(&folder)
            .join("WAD")
            .join("MonkeyKing.wad.client")
            .join("data")
            .join("characters")
            .join("jade_wukong")
            .join("skins")
            .join("skin0.bin")
            .is_file()
    );

    let unknown = ClassicChampion::open(&game, "MonkeyKing")
        .expect("open")
        .with_client_character(Some("Jade_Nobody"));
    assert_eq!(
        unknown.main_character(),
        "jade_monkeyking",
        "a client name absent from the archive is not trusted"
    );
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn test_the_companion_falls_back_only_to_a_real_other_base() {
    let game = standard_game(
        "fallback",
        &[
            (
                wad_path_hash(&skin_bin("zed", 12)),
                companion_bin("Zed", 12),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 0)),
                companion_bin("ZedShadow", 0),
            ),
            (
                wad_path_hash(&skin_bin("zedshadow", 10)),
                companion_bin("ZedShadow", 10),
            ),
        ],
    );
    let champion = StandardChampion::open(&game, "Zed").expect("open");
    assert_eq!(
        champion.companion_source_skin("zedshadow", 12, Some(10)),
        Some(10)
    );
    assert_eq!(
        champion.companion_source_skin("zedshadow", 12, Some(0)),
        None,
        "the default skin is never a fallback"
    );
    assert_eq!(
        champion.companion_source_skin("zedshadow", 12, Some(12)),
        None
    );
    assert_eq!(
        champion.companion_source_skin("zedshadow", 10, None),
        Some(10)
    );
    assert_eq!(champion.companion_source_skin("zedshadow", 12, None), None);
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

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

fn fields_body(fields: &[(u32, u8, Vec<u8>)]) -> Vec<u8> {
    let mut body = (fields.len() as u16).to_le_bytes().to_vec();
    for (name, kind, value) in fields {
        body.extend_from_slice(&name.to_le_bytes());
        body.push(*kind);
        body.extend_from_slice(value);
    }
    body
}

fn skin_with_graph(character: &str, skin: u32, classification: u32) -> Vec<u8> {
    let graph = prop_key_hash(&format!("Characters/{character}/Animations/Skin{skin}"));
    let inner = fields_body(&[(
        prop_key_hash("animationGraphData"),
        0x84,
        graph.to_le_bytes().to_vec(),
    )]);
    let mut embed = 0x1234_0000u32.to_le_bytes().to_vec();
    embed.extend_from_slice(&(inner.len() as u32).to_le_bytes());
    embed.extend_from_slice(&inner);
    let prefix = format!("Characters/{character}/Skins/Skin{skin}");
    let body = fields_body(&[
        (
            prop_key_hash("skinClassification"),
            7,
            classification.to_le_bytes().to_vec(),
        ),
        (
            prop_key_hash("objectPath"),
            17,
            prop_key_hash(&prefix).to_le_bytes().to_vec(),
        ),
        (prop_key_hash("skinAnimationProperties"), 0x83, embed),
    ]);
    serialize_prop_file(&PropFile {
        version: 3,
        links: vec![format!(
            "DATA/Characters/{character}/Animations/Skin{skin}.bin"
        )],
        entries: vec![PropEntry {
            class_hash: 0x9b67_e9f6,
            key_hash: prop_key_hash(&prefix),
            body,
        }],
    })
    .expect("fixture")
}

fn graph_bin(character: &str, skin: u32) -> Vec<u8> {
    let key = prop_key_hash(&format!("Characters/{character}/Animations/Skin{skin}"));
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: 0xf5fb_07c7,
            key_hash: key,
            body: fields_body(&[(prop_key_hash("objectPath"), 17, key.to_le_bytes().to_vec())]),
        }],
    })
    .expect("fixture")
}

fn graph_link_of(bin: &[u8]) -> Option<u32> {
    let body = &parse_prop_file(bin).expect("parse").entries[0].body;
    field_value(
        body,
        &[
            prop_key_hash("skinAnimationProperties"),
            prop_key_hash("animationGraphData"),
        ],
    )
    .expect("walk")
    .and_then(|v| v.as_u32())
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
