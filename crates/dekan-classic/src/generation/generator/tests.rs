use super::*;

pub(super) fn u32_body(value: u32) -> Vec<u8> {
    let mut body = 1u16.to_le_bytes().to_vec();
    body.extend_from_slice(&0x1234_5678u32.to_le_bytes());
    body.push(7);
    body.extend_from_slice(&value.to_le_bytes());
    body
}

pub(super) fn skin_bin_fixture(character: &str, skin: u32) -> Vec<u8> {
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

pub(super) fn skin_object_body(
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

pub(super) fn skin_object_bin(
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

pub(super) fn classified_skin_bin(character: &str, skin: u32, classification: u32) -> Vec<u8> {
    skin_object_bin(character, skin, classification, None)
}

pub(super) fn skin_field(bin: &[u8], name: &str) -> Option<u32> {
    let body = &parse_prop_file(bin).expect("parse").entries[0].body;
    field_value(body, &[prop_key_hash(name)])
        .expect("walk")
        .and_then(|v| v.as_u32())
}

pub(super) fn classification_of(bin: &[u8]) -> u32 {
    skin_field(bin, "skinClassification").expect("classification")
}

pub(super) fn raw_wad(entries: &[(u64, Vec<u8>)]) -> Vec<u8> {
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

pub(super) fn tibbers_bin(skin: u32) -> Vec<u8> {
    let mut prop = parse_prop_file(&skin_bin_fixture("annietibbers", skin)).expect("fixture");
    prop.links
        .push("DATA/Characters/AnnieTibbers/AnnieTibbers.bin".into());
    serialize_prop_file(&prop).expect("fixture")
}

pub(super) fn companion_bin(character: &str, skin: u32) -> Vec<u8> {
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

pub(super) fn standard_game(name: &str, entries: &[(u64, Vec<u8>)]) -> PathBuf {
    let game = std::env::temp_dir().join(format!("dekan_std_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture may not exist yet
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");
    std::fs::write(champions.join("Zed.wad.client"), raw_wad(entries)).expect("write wad");
    game
}

pub(super) fn generated_skin0(mods_dir: &Path, folder: &str, character: &str) -> Option<PropFile> {
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

pub(super) fn fields_body(fields: &[(u32, u8, Vec<u8>)]) -> Vec<u8> {
    let mut body = (fields.len() as u16).to_le_bytes().to_vec();
    for (name, kind, value) in fields {
        body.extend_from_slice(&name.to_le_bytes());
        body.push(*kind);
        body.extend_from_slice(value);
    }
    body
}

pub(super) fn skin_with_graph(character: &str, skin: u32, classification: u32) -> Vec<u8> {
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

pub(super) fn graph_bin(character: &str, skin: u32) -> Vec<u8> {
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

pub(super) fn graph_link_of(bin: &[u8]) -> Option<u32> {
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
fn test_skin_numbers_and_slots_follow_the_client() {
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

#[test]
fn a_generated_skin_is_reused_until_the_game_archive_changes() {
    let game = std::env::temp_dir().join(format!("dekan_std_reuse_{}", std::process::id()));
    let champions = game.join("DATA").join("FINAL").join("Champions");
    std::fs::create_dir_all(&champions).expect("fixture dir");
    let wad_path = champions.join("Zed.wad.client");
    let entry = (
        wad_path_hash(&skin_bin("zed", 1)),
        skin_bin_fixture("Zed", 1),
    );
    std::fs::write(&wad_path, raw_wad(std::slice::from_ref(&entry))).expect("write wad");
    let mods_dir = game.join("mods");
    let build = || {
        StandardChampion::open(&game, "Zed")
            .expect("open")
            .build_mod(1, None, &mods_dir)
            .expect("build")
    };

    let folder = build();
    let marker = mods_dir.join(&folder).join("kept");
    std::fs::write(&marker, b"").expect("marker");
    assert_eq!(build(), folder);
    assert!(marker.is_file());

    let other = (wad_path_hash("data/other.bin"), vec![0u8; 16]);
    std::fs::write(&wad_path, raw_wad(&[entry, other])).expect("rewrite wad");
    assert_eq!(build(), folder);
    assert!(!marker.exists());

    let _ = std::fs::remove_dir_all(&game); // ignore-ok: fixture cleanup
}

#[test]
fn builds_of_one_folder_run_one_at_a_time() {
    let dir = std::env::temp_dir().join(format!("dekan_claim_{}", std::process::id()));
    let inside = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let (dir, inside, peak) = (dir.clone(), inside.clone(), peak.clone());
            std::thread::spawn(move || {
                let _claim = reuse::Claim::wait_for(&dir);
                let now = inside.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
                inside.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("worker");
    }
    assert_eq!(peak.load(std::sync::atomic::Ordering::SeqCst), 1);
}
