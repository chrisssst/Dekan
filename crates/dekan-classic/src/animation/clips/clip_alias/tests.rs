use dekan_wad::prop::tree::Field;
use dekan_wad::prop::{PropEntry, PropFile};

use super::*;

const SKIN_GRAPH: u32 = 0x0000_0044;
const BASE_GRAPH: u32 = 0x0000_0000;
const POINTER: u8 = 0x82;
const LIST: u8 = 0x80;

fn named(name: &str, value: Value) -> Field {
    Field {
        name: h(name),
        value,
    }
}

fn text(value: &str) -> Value {
    let mut bytes = u16::try_from(value.len())
        .expect("short")
        .to_le_bytes()
        .to_vec();
    bytes.extend_from_slice(value.as_bytes());
    Value::Raw {
        kind: FIELD_STRING,
        bytes,
    }
}

fn atomic(track: &str, tick: Option<f32>, sound: Option<&str>) -> Value {
    let mut fields = vec![named("mTrackDataName", hash_value(h(track)))];
    if let Some(tick) = tick {
        fields.push(named(
            "mTickDuration",
            Value::Raw {
                kind: 10,
                bytes: tick.to_le_bytes().to_vec(),
            },
        ));
    }
    if let Some(sound) = sound {
        fields.push(named(
            "mEventDataMap",
            Value::Map {
                key: FIELD_HASH,
                value: POINTER,
                entries: vec![(
                    hash_value(h("swing")),
                    Value::Struct {
                        kind: POINTER,
                        class: h("SoundEventData"),
                        fields: vec![named("mSoundName", text(sound))],
                    },
                )],
            },
        ));
    }
    Value::Struct {
        kind: POINTER,
        class: h("AtomicClipData"),
        fields,
    }
}

fn parallel(children: &[&str]) -> Value {
    Value::Struct {
        kind: POINTER,
        class: h("ParallelClipData"),
        fields: vec![named(
            "mClipNameList",
            Value::List {
                kind: LIST,
                element: FIELD_HASH,
                items: children.iter().map(|c| hash_value(h(c))).collect(),
            },
        )],
    }
}

fn graph(key: u32, clips: Vec<(&str, Value)>) -> Vec<u8> {
    let map = Value::Map {
        key: FIELD_HASH,
        value: POINTER,
        entries: clips
            .into_iter()
            .map(|(name, clip)| (hash_value(h(name)), clip))
            .collect(),
    };
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h("AnimationGraphData"),
            key_hash: key,
            body: tree::write_fields(&[named("mClipDataMap", map)]).expect("fields"),
        }],
    })
    .expect("graph")
}

fn spells() -> Vec<String> {
    ["GarenQ", "GarenW", "GarenE", "GarenR"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn base() -> Vec<u8> {
    graph(
        BASE_GRAPH,
        vec![
            ("Spell2", atomic("Default", None, None)),
            ("Spell3", atomic("Default", None, None)),
            ("Run", atomic("Default", None, None)),
        ],
    )
}

fn spin_skin(sound: Option<&str>) -> Vec<u8> {
    graph(
        SKIN_GRAPH,
        vec![
            ("Spell2", atomic("Default", None, None)),
            ("Run", atomic("Default", None, None)),
            ("Spell3_Normal", parallel(&["Normal_Spin", "Normal_Body"])),
            ("Normal_Spin", atomic("Spell3", Some(0.0208), sound)),
            ("Normal_Body", atomic("Default", Some(0.0208), None)),
            ("Spell3_Med", parallel(&["Med_Spin", "Med_Body"])),
            ("Med_Spin", atomic("Spell3", Some(0.0185), sound)),
            ("Med_Body", atomic("Default", Some(0.0185), None)),
            ("Spell3_Fast", parallel(&["Fast_Spin", "Fast_Body"])),
            ("Fast_Spin", atomic("Spell3", Some(0.0164), sound)),
            ("Fast_Body", atomic("Default", Some(0.0164), None)),
        ],
    )
}

fn alias(skin: &[u8]) -> Option<AliasedGraph> {
    alias_missing_clips(skin, SKIN_GRAPH, &base(), BASE_GRAPH, &spells()).expect("alias")
}

fn clips(bin: &[u8]) -> Vec<(u32, Value)> {
    let file = parse_prop_file(bin).expect("parse");
    let mut fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    clip_entries(&mut fields)
        .expect("clip map")
        .iter()
        .map(|(k, v)| (k.as_u32().expect("key"), v.clone()))
        .collect()
}

#[test]
fn test_a_missing_spell_clip_takes_its_variant_at_normal_speed() {
    let skin = spin_skin(Some("Play_sfx_GarenSkin13_GarenE_swing"));
    let (out, aliases) = alias(&skin).expect("aliased");
    assert_eq!(
        aliases,
        vec![ClipAlias {
            missing: h("Spell3"),
            variant: h("Spell3_Normal"),
            variants: 3,
        }]
    );
    let after = clips(&out);
    let find = |key: u32| &after.iter().find(|(k, _)| *k == key).expect("clip").1;
    assert_eq!(find(h("Spell3")), find(h("Spell3_Normal")));
    let before = clips(&skin);
    assert_eq!(
        &after[..before.len()],
        &before[..],
        "the game's clips stay as they were"
    );
}

#[test]
fn test_variants_without_the_spell_sound_are_not_trusted() {
    assert_eq!(alias(&spin_skin(None)), None);
    assert_eq!(
        alias(&spin_skin(Some("Play_sfx_GarenSkin13_Respawn"))),
        None
    );
}

#[test]
fn test_loose_clips_on_the_track_are_not_variants() {
    let turn = graph(
        SKIN_GRAPH,
        vec![
            ("TurnL", atomic("Spell3", Some(0.03), Some("GarenE"))),
            ("TurnR", atomic("Spell3", Some(0.03), Some("GarenE"))),
        ],
    );
    assert_eq!(alias(&turn), None);
}

#[test]
fn test_a_single_variant_is_not_enough_evidence() {
    let one = graph(
        SKIN_GRAPH,
        vec![
            ("Spell3_Only", parallel(&["Only_Spin"])),
            ("Only_Spin", atomic("Spell3", Some(0.02), Some("GarenE"))),
        ],
    );
    assert_eq!(alias(&one), None);
}

#[test]
fn test_variants_used_by_other_clips_are_left_alone() {
    let used = graph(
        SKIN_GRAPH,
        vec![
            ("Combo", parallel(&["Spell3_A", "Spell3_B"])),
            ("Spell3_A", parallel(&["A_Spin"])),
            ("A_Spin", atomic("Spell3", Some(0.02), Some("GarenE"))),
            ("Spell3_B", parallel(&["B_Spin"])),
            ("B_Spin", atomic("Spell3", Some(0.03), Some("GarenE"))),
        ],
    );
    assert_eq!(alias(&used), None);
}

#[test]
fn test_a_skin_with_every_spell_clip_is_unchanged() {
    let full = graph(
        SKIN_GRAPH,
        vec![
            ("Spell2", atomic("Default", None, None)),
            ("Spell3", atomic("Default", None, None)),
        ],
    );
    assert_eq!(alias(&full), None);
}

#[test]
fn test_spell_names_come_from_the_character_record() {
    let record = serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h("CharacterRecord"),
            key_hash: 1,
            body: tree::write_fields(&[named(
                "spellNames",
                Value::List {
                    kind: LIST,
                    element: FIELD_STRING,
                    items: vec![text("GarenQAbility/GarenQ"), text("GarenE")],
                },
            )])
            .expect("record"),
        }],
    })
    .expect("bin");
    assert_eq!(spell_names(&record), vec!["GarenQ", "GarenE"]);
}
