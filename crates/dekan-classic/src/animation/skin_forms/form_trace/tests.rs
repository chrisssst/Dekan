use dekan_wad::prop::{PropEntry, PropFile, serialize_prop_file};
use dekan_wad::writer::{WadWriter, optimal_raw};

use super::*;

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

fn string(text: &str) -> Value {
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

fn hash_list(hashes: &[u32]) -> Value {
    Value::List {
        kind: 0x80,
        element: 0x11,
        items: hashes
            .iter()
            .map(|v| Value::Raw {
                kind: 0x11,
                bytes: v.to_le_bytes().to_vec(),
            })
            .collect(),
    }
}

fn field(name: &str, value: Value) -> tree::Field {
    tree::Field {
        name: h(name),
        value,
    }
}

fn pointer(class: &str, fields: Vec<tree::Field>) -> Value {
    Value::Struct {
        kind: 0x82,
        class: h(class),
        fields,
    }
}

fn skn(parts: &[&str]) -> Vec<u8> {
    let mut bytes = SKN_MAGIC.to_vec();
    bytes.extend_from_slice(&[1, 0, 1, 0]);
    bytes.extend_from_slice(&u32::try_from(parts.len()).expect("count").to_le_bytes());
    for part in parts {
        let mut name = part.as_bytes().to_vec();
        name.resize(SKN_PART_SIZE, 0);
        bytes.extend_from_slice(&name);
    }
    bytes
}

fn prop(key: u32, class: &str, fields: &[tree::Field]) -> Vec<u8> {
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h(class),
            key_hash: key,
            body: tree::write_fields(fields).expect("body"),
        }],
    })
    .expect("prop")
}

fn graph(clips: Vec<(u32, Value)>) -> Vec<u8> {
    prop(
        7,
        "AnimationGraphData",
        &[field(
            "mClipDataMap",
            Value::Map {
                key: 0x11,
                value: 0x82,
                entries: clips
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            Value::Raw {
                                kind: 0x11,
                                bytes: k.to_le_bytes().to_vec(),
                            },
                            v,
                        )
                    })
                    .collect(),
            },
        )],
    )
}

fn clip_with_event(show: &[u32], hide: &[u32]) -> Value {
    pointer(
        "AtomicClipData",
        vec![field(
            "mEventDataMap",
            Value::Map {
                key: 0x11,
                value: 0x82,
                entries: vec![(
                    Value::Raw {
                        kind: 0x11,
                        bytes: 1u32.to_le_bytes().to_vec(),
                    },
                    pointer(
                        "SubmeshVisibilityEventData",
                        vec![
                            field("mShowSubmeshList", hash_list(show)),
                            field("mHideSubmeshList", hash_list(hide)),
                        ],
                    ),
                )],
            },
        )],
    )
}

#[test]
fn test_a_skin_mesh_names_its_parts_and_a_broken_mesh_names_none() {
    assert_eq!(
        skn_parts(&skn(&["Base_Sword", "Tank_Sword"])),
        vec!["Base_Sword", "Tank_Sword"]
    );
    assert!(skn_parts(b"not a mesh").is_empty());
    assert!(skn_parts(&SKN_MAGIC).is_empty());
}

#[test]
fn test_the_trace_reads_forms_game_events_and_the_added_chain() {
    let dir = std::env::temp_dir().join(format!(
        "dekan_form_trace_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let base = h("Base_Sword");
    let tank = h("Tank_Sword");
    let mesh = "assets/characters/zed/skins/skin1/zed.skn";
    let skin_bin = prop(
        1,
        "SkinCharacterDataProperties",
        &[field("simpleSkin", string(mesh))],
    );
    let graph_path = "data/characters/zed/animations/skin1.bin";
    let original = graph(vec![
        (10, clip_with_event(&[base], &[tank])),
        (11, pointer("AtomicClipData", Vec::new())),
    ]);
    let mut writer = WadWriter::default();
    writer.insert(
        wad_path_hash(mesh),
        optimal_raw(skn(&["Base_Sword", "Tank_Sword"])).expect("skn"),
    );
    writer.insert(
        wad_path_hash(graph_path),
        optimal_raw(original).expect("graph"),
    );
    let wad_path = dir.join("Zed.wad.client");
    std::fs::write(&wad_path, writer.to_bytes().expect("wad")).expect("wad");
    let wad = WadFile::open(&wad_path).expect("open");

    let condition = pointer(
        "ConditionBoolClipData",
        vec![
            field(
                "Updater",
                pointer(
                    "LogicDriverBoolParametricUpdater",
                    vec![field(
                        "driver",
                        pointer(
                            "SubmeshVisibilityBoolDriver",
                            vec![field(
                                "Submeshes",
                                Value::Raw {
                                    kind: 0x11,
                                    bytes: tank.to_le_bytes().to_vec(),
                                },
                            )],
                        ),
                    )],
                ),
            ),
            field(
                "mTrueConditionClipName",
                Value::Raw {
                    kind: 0x11,
                    bytes: 12u32.to_le_bytes().to_vec(),
                },
            ),
            field("mClipNameList", hash_list(&[10, 11])),
            field(
                "mUnused",
                Value::Optional {
                    inner: 0x82,
                    value: None,
                },
            ),
        ],
    );
    let generated = graph(vec![
        (10, clip_with_event(&[base], &[tank])),
        (11, pointer("AtomicClipData", Vec::new())),
        (12, clip_with_event(&[tank], &[base])),
        (13, condition),
    ]);
    let swaps = vec![
        GearSwap {
            show: vec![base],
            hide: vec![tank],
            equip: Some("Toggle_Base".into()),
            transition: None,
        },
        GearSwap {
            show: vec![tank],
            hide: vec![base],
            equip: None,
            transition: None,
        },
    ];
    let form = FormTrace {
        wad: &wad,
        alias: "Zed",
        skin_bin: &skin_bin,
        graph_path,
        graph_key: 7,
        generated_graph: &generated,
        swaps: &swaps,
        markers: &[base, tank],
    };

    trace(&form);
    tracing::subscriber::with_default(EveryLevel, || trace(&form));

    let names = part_names(&wad, &skin_bin);
    assert_eq!(names.get(&tank).map(String::as_str), Some("Tank_Sword"));
    let mut watched = Vec::new();
    driven_parts(&clips(&generated, 7)[&13], &mut watched);
    assert_eq!(watched, vec![tank]);
    let mut refs = Vec::new();
    clip_refs(&clips(&generated, 7)[&13], &mut refs);
    assert_eq!(refs, vec![12, 10, 11]);
    assert_eq!(
        visibility_events(&clips(&generated, 7)[&12]),
        vec![(vec![tank], vec![base])]
    );
    assert!(clips(b"not a bin", 7).is_empty());
    assert!(clips(&generated, 99).is_empty());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup
}
