use super::*;
use crate::audio::sound_bank::tests::bank;
use crate::vfx_markers::embed;
use dekan_wad::prop::{PropEntry, PropFile};

fn container(group_kind: u8, group: u32, default: u32, children: &[u32]) -> Vec<u8> {
    let mut payload = vec![0; 12];
    payload.push(group_kind);
    payload.extend_from_slice(&group.to_le_bytes());
    payload.extend_from_slice(&default.to_le_bytes());
    for child in children {
        payload.extend_from_slice(&child.to_le_bytes());
    }
    payload
}

fn elemental_bank() -> Vec<u8> {
    bank(&[
        (
            SWITCH_CONTAINER,
            1,
            container(0, 0xA2CC, gear_value(0), &[gear_value(1), gear_value(2)]),
        ),
        (
            SWITCH_CONTAINER,
            2,
            container(0, 0xBFE2, gear_value(0), &[gear_value(2)]),
        ),
        (
            SWITCH_CONTAINER,
            3,
            container(1, 0x5747, gear_value(0), &[]),
        ),
        (
            SWITCH_CONTAINER,
            4,
            container(0, 0x0001, wwise_id("Day"), &[wwise_id("Night")]),
        ),
        (EVENT, 5, vec![0]),
    ])
}

#[test]
fn only_switch_groups_that_choose_by_gear_are_found() {
    let objects = sound_bank::parse(&elemental_bank())
        .expect("bank")
        .objects()
        .expect("objects");
    assert_eq!(gear_groups(&objects, 3), BTreeSet::from([0xA2CC, 0xBFE2]));
}

#[test]
fn each_form_gets_an_event_that_sets_every_gear_group_to_its_value() {
    let added = add_form_switches(&elemental_bank(), 3)
        .expect("add")
        .expect("groups");
    assert_eq!(
        added.events,
        [
            "Dekan_Form0_Sound",
            "Dekan_Form1_Sound",
            "Dekan_Form2_Sound"
        ]
    );
    let objects = sound_bank::parse(&added.bank)
        .expect("bank")
        .objects()
        .expect("objects");
    assert_eq!(
        objects.len(),
        5 + 3 * 3,
        "two actions and one event per form"
    );
    let event = objects
        .iter()
        .find(|o| o.kind == EVENT && o.id == wwise_id("Dekan_Form2_Sound"))
        .expect("form 2 event");
    assert_eq!(event.payload[0], 2);
    let first = u32_le(&event.payload, 1).expect("action");
    let action = objects.iter().find(|o| o.id == first).expect("action");
    assert_eq!(action.kind, ACTION);
    assert_eq!(action.payload, set_switch(0xA2CC, gear_value(2)));
    assert_eq!(action.payload.len(), 17);
    assert_eq!(&action.payload[..2], &[0x01, 0x19]);
}

#[test]
fn unknown_versions_banks_without_gear_and_id_clashes_are_left_alone_or_refused() {
    let mut old = elemental_bank();
    old[8..12].copy_from_slice(&134u32.to_le_bytes());
    assert!(add_form_switches(&old, 3).expect("old").is_none());
    let plain = bank(&[(SWITCH_CONTAINER, 1, container(0, 7, wwise_id("Day"), &[]))]);
    assert!(add_form_switches(&plain, 3).expect("plain").is_none());
    let clash = bank(&[
        (
            SWITCH_CONTAINER,
            1,
            container(0, 7, gear_value(0), &[gear_value(1)]),
        ),
        (EVENT, wwise_id("Dekan_Form1_Sound"), vec![0]),
    ]);
    assert!(add_form_switches(&clash, 2).is_err());
}

fn bin(class: &str, key: u32, fields: Vec<tree::Field>) -> Vec<u8> {
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h(class),
            key_hash: key,
            body: tree::write_fields(&fields).expect("body"),
        }],
    })
    .expect("bin")
}

fn clip_events(graph: &[u8], key: u32) -> Vec<String> {
    let file = parse_prop_file(graph).expect("graph");
    let fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    let Some(Value::Map { entries, .. }) = tree::field(&fields, h("mClipDataMap")) else {
        panic!("clips");
    };
    let clip = &entries
        .iter()
        .find(|(k, _)| k.as_u32() == Some(key))
        .expect("clip")
        .1;
    match clip
        .fields()
        .and_then(|f| tree::field(f, h("mEventDataMap")))
    {
        Some(Value::Map { entries, .. }) => entries
            .iter()
            .filter_map(|(_, e)| {
                e.fields()
                    .and_then(|f| tree::field(f, h("mSoundName")))
                    .cloned()
            })
            .filter_map(|v| match v {
                Value::Raw { bytes, .. } => Some(String::from_utf8_lossy(&bytes[2..]).into_owned()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[test]
fn entering_a_form_plays_its_sound_event_and_other_clips_stay_as_they_were() {
    let atomic = || pointer("AtomicClipData", Vec::new());
    let graph = bin(
        "AnimationGraphData",
        9,
        vec![named(
            "mClipDataMap",
            Value::Map {
                key: FIELD_HASH,
                value: FIELD_POINTER,
                entries: vec![
                    (hash_value(crate::form_state::state_key(0)), atomic()),
                    (hash_value(crate::form_state::kick_key(1)), atomic()),
                    (hash_value(h("Run")), atomic()),
                ],
            },
        )],
    );
    let events = [
        "Dekan_Form0_Sound".to_owned(),
        "Dekan_Form1_Sound".to_owned(),
    ];
    let fired = fire_on_entry(&graph, 9, &events).expect("fire");
    assert_eq!(
        clip_events(&fired, crate::form_state::state_key(0)),
        ["Dekan_Form0_Sound"]
    );
    assert_eq!(
        clip_events(&fired, crate::form_state::kick_key(1)),
        ["Dekan_Form1_Sound"]
    );
    assert!(clip_events(&fired, h("Run")).is_empty());
    assert!(fire_on_entry(&graph, 77, &events).is_err());
}

#[test]
fn the_new_events_join_the_list_of_the_unit_that_loads_the_bank() {
    let unit = |bank: &str, events: &[&str]| {
        embed(
            "SkinAudioBankUnit",
            vec![
                named(
                    "bankPath",
                    Value::List {
                        kind: 0x80,
                        element: 16,
                        items: vec![text(bank)],
                    },
                ),
                named(
                    "events",
                    Value::List {
                        kind: 0x80,
                        element: 16,
                        items: events.iter().map(|e| text(e)).collect(),
                    },
                ),
            ],
        )
    };
    let skin = bin(
        "SkinCharacterDataProperties",
        1,
        vec![named(
            "skinAudioProperties",
            embed(
                "SkinAudioProperties",
                vec![named(
                    "bankUnits",
                    Value::List {
                        kind: 0x80,
                        element: 0x83,
                        items: vec![
                            unit("A/SFX_events.bnk", &["Play_a"]),
                            unit("A/VO_events.bnk", &["Play_vo"]),
                        ],
                    },
                )],
            ),
        )],
    );
    let listed =
        list_events(&skin, "a/sfx_events.bnk", &["Dekan_Form1_Sound".into()]).expect("list");
    let file = parse_prop_file(&listed).expect("skin");
    let fields = tree::parse_fields(&file.entries[0].body).expect("fields");
    let units = tree::field(&fields, h("skinAudioProperties"))
        .and_then(Value::fields)
        .and_then(|a| tree::field(a, h("bankUnits")))
        .and_then(Value::items)
        .expect("units");
    let names = |i: usize| {
        units[i]
            .fields()
            .and_then(|f| tree::field(f, h("events")))
            .and_then(Value::items)
            .map_or(0, <[Value]>::len)
    };
    assert_eq!((names(0), names(1)), (2, 1));
}
