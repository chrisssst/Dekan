use dekan_wad::prop::{PropEntry, PropFile};

use super::*;
use crate::gear_toggle::{FIELD_HASH, named, pointer};
use crate::vfx_markers::{FIELD_POINTER, raw};

const GRAPH: u32 = 7;
const FIELD_BOOL: u8 = 1;
const SLASH_RED: u32 = 0xA0;
const SLASH_BLUE: u32 = 0xA1;

fn fires(key: &str) -> Value {
    pointer(
        "ParticleEventData",
        vec![named("mEffectKey", hash_value(h(key)))],
    )
}

fn kills_effect(key: &str) -> Value {
    pointer(
        "ParticleEventData",
        vec![
            named("mEffectKey", hash_value(h(key))),
            named("mIsKillEvent", raw(FIELD_BOOL, vec![1])),
        ],
    )
}

fn map(entries: Vec<(u32, Value)>) -> Value {
    Value::Map {
        key: FIELD_HASH,
        value: FIELD_POINTER,
        entries: entries
            .into_iter()
            .map(|(k, v)| (hash_value(k), v))
            .collect(),
    }
}

fn clip(events: &[(&str, Value)]) -> Value {
    pointer(
        "AtomicClipData",
        vec![named(
            "mEventDataMap",
            map(events.iter().map(|(n, e)| (h(n), e.clone())).collect()),
        )],
    )
}

fn graph(clips: Vec<(u32, Value)>) -> Vec<u8> {
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries: vec![PropEntry {
            class_hash: h("AnimationGraphData"),
            key_hash: GRAPH,
            body: tree::write_fields(&[named("mClipDataMap", map(clips))]).expect("graph"),
        }],
    })
    .expect("bin")
}

fn two_forms(name: &str, events: &[(&str, Value)]) -> Vec<(u32, Value)> {
    vec![
        (h(name), pointer("ConditionBoolClipData", Vec::new())),
        (copy_key(0, h(name)), clip(events)),
        (copy_key(1, h(name)), clip(events)),
    ]
}

fn fired_by(out: &[u8], clip: u32) -> Vec<u32> {
    let file = parse_prop_file(out).expect("prop");
    let graph = tree::parse_fields(&file.entries[0].body).expect("graph");
    let Some(Value::Map { entries, .. }) = tree::field(&graph, h("mClipDataMap")) else {
        panic!("the graph lost its clips");
    };
    let (_, clip) = entries
        .iter()
        .find(|(k, _)| k.as_u32() == Some(clip))
        .expect("clip");
    particle_events(clip).filter_map(effect_key).collect()
}

fn slash(blue: Option<u32>) -> FormKeys {
    FormKeys::from([(h("Slash"), vec![Some(SLASH_RED), blue])])
}

#[test]
fn test_each_forms_copy_of_a_clip_fires_that_forms_own_system() {
    let events = [("Hit", fires("Slash")), ("Step", fires("Dust"))];
    let bin = graph(two_forms("Attack1", &events));
    let (out, fired) = fire_own_effects(&bin, GRAPH, &slash(Some(SLASH_BLUE)))
        .expect("graph")
        .expect("changed");
    assert_eq!(fired, 2);
    assert_eq!(
        fired_by(&out, copy_key(0, h("Attack1"))),
        [SLASH_RED, h("Dust")],
        "an effect no form changes keeps its key"
    );
    assert_eq!(
        fired_by(&out, copy_key(1, h("Attack1"))),
        [SLASH_BLUE, h("Dust")]
    );
}

#[test]
fn test_an_effect_some_clip_kills_by_key_stays_on_the_shared_system() {
    let mut clips = two_forms("Attack1", &[("Hit", fires("Slash"))]);
    clips.extend(two_forms("Run", &[("Stop", kills_effect("Slash"))]));
    assert!(
        fire_own_effects(&graph(clips), GRAPH, &slash(Some(SLASH_BLUE)))
            .expect("graph")
            .is_none(),
        "a kill in another form's copy would miss an effect fired under a per-form key"
    );
}

#[test]
fn test_a_form_without_a_system_of_its_own_keeps_the_shared_key() {
    let bin = graph(two_forms("Attack1", &[("Hit", fires("Slash"))]));
    let (out, fired) = fire_own_effects(&bin, GRAPH, &slash(None))
        .expect("graph")
        .expect("changed");
    assert_eq!(fired, 1);
    assert_eq!(fired_by(&out, copy_key(0, h("Attack1"))), [SLASH_RED]);
    assert_eq!(fired_by(&out, copy_key(1, h("Attack1"))), [h("Slash")]);
}

#[test]
fn test_graphs_without_form_copies_or_form_effects_are_left_alone() {
    let copies = graph(two_forms("Attack1", &[("Hit", fires("Slash"))]));
    assert!(
        fire_own_effects(&copies, GRAPH, &FormKeys::new())
            .expect("graph")
            .is_none()
    );
    let plain = graph(vec![(h("Attack1"), clip(&[("Hit", fires("Slash"))]))]);
    assert!(
        fire_own_effects(&plain, GRAPH, &slash(Some(SLASH_BLUE)))
            .expect("graph")
            .is_none(),
        "a clip that is not a form's copy belongs to no form"
    );
    assert!(
        fire_own_effects(&copies, GRAPH + 1, &slash(Some(SLASH_BLUE)))
            .expect("graph")
            .is_none()
    );
}
