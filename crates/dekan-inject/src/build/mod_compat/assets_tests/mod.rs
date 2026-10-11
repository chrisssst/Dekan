use super::tests::*;
use super::*;

#[test]
fn test_asset_paths_renamed_by_the_game_are_relinked_and_the_mods_own_are_kept() {
    let dir = temp("assets");
    let renamed = "ASSETS/Characters/Zed/Skins/Skin1/Particles/Zed_Glow.SKINS_Zed_Skin1.tex";
    let in_game = "ASSETS/Characters/Zed/Skins/Skin1/Particles/Zed_Glow.tex";
    let twin = "ASSETS/Characters/Zed/Skins/Skin1/Particles/Zed_Trail.dds";
    let twin_in_game = "ASSETS/Characters/Zed/Skins/Skin1/Particles/Zed_Trail.tex";
    let own = "ASSETS/Characters/Zed/Skins/Skin1/Particles/Custom.SKINS_Zed_Skin1.tex";
    let lost = "ASSETS/Characters/Zed/Skins/Skin1/Particles/Gone.tex";
    let chroma = "ASSETS/Shared/Particles/Ring.Chroma_Zed_Red.tex";
    let chroma_in_game = "ASSETS/Shared/Particles/Ring.tex";

    let game_path = dir.join("game").join("Zed.wad.client");
    write_wad(
        &game_path,
        &[
            (wad_path_hash(&in_game.to_ascii_lowercase()), b"t".to_vec()),
            (
                wad_path_hash(&twin_in_game.to_ascii_lowercase()),
                b"t".to_vec(),
            ),
            (
                wad_path_hash(&own.replace(".SKINS_Zed_Skin1", "").to_ascii_lowercase()),
                b"t".to_vec(),
            ),
            (
                wad_path_hash(&chroma_in_game.to_ascii_lowercase()),
                b"t".to_vec(),
            ),
        ],
    );
    let game = BTreeMap::from([(
        "zed".to_owned(),
        GameWad {
            relpath: PathBuf::from("DATA/FINAL/Champions/Zed.wad.client"),
            path: game_path.clone(),
            names: {
                let mut names: Vec<u64> = WadFile::open(&game_path)
                    .expect("game")
                    .toc()
                    .map(|e| e.path_hash)
                    .collect();
                names.sort_unstable();
                names
            },
        },
    )]);

    let body = write_fields(&[
        string_field(1, renamed),
        string_field(2, twin),
        string_field(3, own),
        string_field(4, lost),
        string_field(5, chroma),
        dekan_wad::prop::tree::Field {
            name: 6,
            value: Value::Struct {
                kind: 0x83,
                class: 9,
                fields: Vec::new(),
            },
        },
    ])
    .expect("body");
    let prop = dekan_wad::prop::PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![dekan_wad::prop::PropEntry {
            class_hash: 0x1111,
            key_hash: 0x2222,
            body,
        }],
    };
    let wad = dir.join("mod").join("Zed.wad.client");
    write_wad(
        &wad,
        &[
            (
                wad_path_hash("data/characters/zed/skins/skin0.bin"),
                serialize_prop_file(&prop).expect("prop"),
            ),
            (wad_path_hash(&own.to_ascii_lowercase()), b"mine".to_vec()),
        ],
    );

    let relinks = relink_wad(&wad, &game, &game_hash_set(&game)).expect("repair");
    assert_eq!(relinks.len(), 3, "{relinks:?}");
    let after = WadFile::open(&wad).expect("open");
    let fixed = parse_prop_file(
        &after
            .read(wad_path_hash("data/characters/zed/skins/skin0.bin"))
            .expect("read")
            .expect("skin0"),
    )
    .expect("prop");
    assert_eq!(
        strings_of(&fixed),
        vec![in_game, twin_in_game, own, lost, chroma_in_game]
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

#[test]
fn test_asset_formats_are_read_from_their_headers() {
    assert_eq!(
        asset_format(&skn(4, 1)),
        Some(AssetFormat {
            kind: "skn",
            version: (4 << 16) | 1
        })
    );
    let mut anm = b"r3d2anmd".to_vec();
    anm.extend_from_slice(&5u32.to_le_bytes());
    assert_eq!(
        asset_format(&anm),
        Some(AssetFormat {
            kind: "anm",
            version: 5
        })
    );
    let mut skl = 1234u32.to_le_bytes().to_vec();
    skl.extend_from_slice(&0x22FD_4FC3u32.to_le_bytes());
    skl.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        asset_format(&skl),
        Some(AssetFormat {
            kind: "skl",
            version: 0
        })
    );
    assert_eq!(
        asset_format(&[b"PROP".as_slice(), &[3, 0, 0, 0, 0, 0, 0, 0]].concat()),
        None
    );
    assert_eq!(asset_format(b"r3d"), None);
}

#[test]
fn test_a_format_the_game_no_longer_uses_is_reported_and_others_are_not() {
    let dir = temp("formats");
    let game_path = dir.join("game").join("Zed.wad.client");
    write_wad(&game_path, &[(1, skn(4, 1))]);
    let mut names = vec![1u64];
    names.sort_unstable();
    let game = BTreeMap::from([(
        "zed".to_owned(),
        GameWad {
            relpath: PathBuf::from("DATA/FINAL/Champions/Zed.wad.client"),
            path: game_path,
            names,
        },
    )]);
    let hashes = game_hash_set(&game);
    let mut anm = b"r3d2anmd".to_vec();
    anm.extend_from_slice(&3u32.to_le_bytes());
    let wad = dir.join("mod").join("Zed.wad.client");
    write_wad(&wad, &[(10, skn(2, 1)), (11, skn(4, 1)), (12, anm)]);

    let found = Repairer::new(&game, &hashes)
        .unknown_formats(&wad)
        .expect("formats");
    assert_eq!(
        found.into_iter().collect::<Vec<_>>(),
        vec![AssetFormat {
            kind: "skn",
            version: (2 << 16) | 1
        }],
        "a kind the game archive does not hold is not judged"
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}

fn with_version(magic: &[u8], at: usize, version: &[u8]) -> Vec<u8> {
    let mut head = magic.to_vec();
    head.resize(at, 0);
    head.extend_from_slice(version);
    head.resize(16, 0);
    head
}

#[test]
fn test_every_known_asset_container_is_identified() {
    let cases = [
        (
            with_version(b"r3d2canm", 8, &[4, 0, 0, 0]),
            "anm-compressed",
            4,
        ),
        (
            with_version(b"r3d2Mesh", 8, &[2, 0, 1, 0]),
            "scb",
            (2 << 16) | 1,
        ),
        (with_version(&[b'T', b'E', b'X', 0], 4, &[]), "tex", 0),
        (with_version(b"DDS ", 4, &[]), "dds", 0),
        (with_version(b"BKHD", 8, &[145, 0, 0, 0]), "bnk", 145),
        (with_version(b"r3d2", 4, &[1, 0, 0, 0]), "wpk", 1),
        (with_version(b"r3d2sklt", 8, &[2, 0, 0, 0]), "skl", 2),
    ];
    for (head, kind, version) in cases {
        assert_eq!(
            asset_format(&head),
            Some(AssetFormat { kind, version }),
            "{kind}"
        );
    }
    assert_eq!(
        AssetFormat {
            kind: "skn",
            version: 3
        }
        .to_string(),
        "skn v3"
    );
}

fn string_value(text: &str) -> Value {
    let mut bytes = u16::try_from(text.len())
        .expect("len")
        .to_le_bytes()
        .to_vec();
    bytes.extend_from_slice(text.as_bytes());
    Value::Raw {
        kind: FIELD_STRING,
        bytes,
    }
}

#[test]
fn test_asset_paths_nested_in_lists_structs_options_and_maps_are_relinked() {
    let old = "ASSETS/Shared/Particles/Glow.Chroma_Zed_Red.tex";
    let new = "ASSETS/Shared/Particles/Glow.tex";
    let mut value = Value::List {
        kind: 0x80,
        element: 0x83,
        items: vec![Value::Struct {
            kind: 0x83,
            class: 7,
            fields: vec![dekan_wad::prop::tree::Field {
                name: 1,
                value: Value::Optional {
                    inner: 0x86,
                    value: Some(Box::new(Value::Map {
                        key: FIELD_STRING,
                        value: FIELD_STRING,
                        entries: vec![(string_value(old), string_value(old))],
                    })),
                },
            }],
        }],
    };
    let mut relinks = Vec::new();
    relink_assets(&mut value, &|_| false, &|path| path == new, &mut relinks);
    assert_eq!(relinks.len(), 2, "the key and the value of the map");
    let expected = Value::List {
        kind: 0x80,
        element: 0x83,
        items: vec![Value::Struct {
            kind: 0x83,
            class: 7,
            fields: vec![dekan_wad::prop::tree::Field {
                name: 1,
                value: Value::Optional {
                    inner: 0x86,
                    value: Some(Box::new(Value::Map {
                        key: FIELD_STRING,
                        value: FIELD_STRING,
                        entries: vec![(string_value(new), string_value(new))],
                    })),
                },
            }],
        }],
    };
    assert_eq!(value, expected);
}

#[test]
fn test_formats_are_read_from_an_unpacked_wad_folder() {
    let dir = temp("formats_folder");
    let game_path = dir.join("game").join("Zed.wad.client");
    write_wad(&game_path, &[(1, skn(4, 1))]);
    let game = BTreeMap::from([(
        "zed".to_owned(),
        GameWad {
            relpath: PathBuf::from("DATA/FINAL/Champions/Zed.wad.client"),
            path: game_path,
            names: vec![1],
        },
    )]);
    let hashes = game_hash_set(&game);
    let folder = dir.join("mod").join("WAD").join("Zed.wad.client");
    let mesh = folder.join("assets/characters/zed/skins/skin1/zed.skn");
    std::fs::create_dir_all(mesh.parent().expect("parent")).expect("dir");
    std::fs::write(&mesh, skn(1, 1)).expect("skn");

    let found = Repairer::new(&game, &hashes)
        .unknown_formats(&folder)
        .expect("formats");
    assert_eq!(
        found.into_iter().collect::<Vec<_>>(),
        vec![AssetFormat {
            kind: "skn",
            version: (1 << 16) | 1
        }]
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}
