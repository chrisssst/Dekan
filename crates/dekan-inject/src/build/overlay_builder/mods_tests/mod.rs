use super::tests::{TempDir, game, make_mod, read, write_wad};
use super::*;
use dekan_wad::hash::content_checksum;

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
