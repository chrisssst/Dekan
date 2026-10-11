use super::*;
use crate::gear_toggle::{FIELD_HASH, hash_value, named, pointer};
use crate::vfx_markers::{embed, text};
use dekan_wad::prop::{PropEntry, PropFile, serialize_prop_file};

const GRAPH: u32 = 9;

fn sound(name: &str) -> Value {
    pointer("SoundEventData", vec![named("mSoundName", text(name))])
}

fn clip(sounds: &[&str]) -> Value {
    pointer(
        "AtomicClipData",
        vec![named(
            "mEventDataMap",
            Value::Map {
                key: FIELD_HASH,
                value: 0x82,
                entries: sounds
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (hash_value(u32::try_from(i).expect("i")), sound(s)))
                    .collect(),
            },
        )],
    )
}

fn bin(entries: Vec<PropEntry>) -> Vec<u8> {
    serialize_prop_file(&PropFile {
        version: 3,
        links: Vec::new(),
        entries,
    })
    .expect("bin")
}

fn graph(clips: Vec<(u32, Value)>) -> Vec<u8> {
    bin(vec![PropEntry {
        class_hash: h("AnimationGraphData"),
        key_hash: GRAPH,
        body: tree::write_fields(&[named(
            "mClipDataMap",
            Value::Map {
                key: FIELD_HASH,
                value: 0x82,
                entries: clips.into_iter().map(|(k, v)| (hash_value(k), v)).collect(),
            },
        )])
        .expect("graph"),
    }])
}

fn situations(keys: &[u32]) -> Vec<u8> {
    let rule = embed("ContextualSituation", Vec::new());
    bin(vec![PropEntry {
        class_hash: h("ContextualActionData"),
        key_hash: 1,
        body: tree::write_fields(&[named(
            "mSituations",
            Value::Map {
                key: FIELD_HASH,
                value: 0x83,
                entries: keys
                    .iter()
                    .map(|k| (hash_value(*k), rule.clone()))
                    .collect(),
            },
        )])
        .expect("cac"),
    }])
}

#[test]
fn a_form_token_is_its_character_name_without_the_champion() {
    assert_eq!(
        form_token(
            "Lux",
            "ASSETS/Characters/LuxAir/Skins/Skin07/Lux_Air_Skin07.skn"
        )
        .as_deref(),
        Some("air")
    );
    assert_eq!(
        form_token("Lux", "ASSETS/Characters/Lux/Skins/Skin07/Lux_Skin07.skn"),
        None
    );
    assert_eq!(
        form_token("Lux", "ASSETS/Characters/Ahri/Skins/Skin01/Ahri.skn"),
        None
    );
    assert_eq!(form_token("Lux", "no/character/folder.skn"), None);
}

#[test]
fn each_form_takes_the_situation_clip_whose_sounds_name_it_and_nothing_ambiguous() {
    let bin = graph(vec![
        (
            10,
            clip(&[
                "Play_sfx_Lux_Transform_Air_buffactivate",
                "Play_vo_LuxSkin07_Air_Unique2DTransform",
            ]),
        ),
        (11, clip(&["Play_sfx_Lux_Transform_Storm_buffactivate"])),
        (12, clip(&["Play_sfx_Lux_Airborne_Hit"])),
        (13, clip(&["Play_sfx_Lux_Transform_Fire_buffactivate"])),
        (14, clip(&["Play_sfx_Lux_Transform_Fire_UI"])),
        (15, clip(&["Play_sfx_Lux_Transform_Ice_buffactivate"])),
    ]);
    let cac = situation_keys(&situations(&[10, 11, 12, 13, 14])).expect("situations");
    let tokens = [
        None,
        Some("air".to_owned()),
        Some("storm".to_owned()),
        Some("fire".to_owned()),
        Some("ice".to_owned()),
        Some("water".to_owned()),
    ];
    assert_eq!(
        transition_clips(&bin, GRAPH, &cac, &tokens).expect("transitions"),
        [None, Some(10), Some(11), None, None, None],
        "two fire clips are ambiguous, ice is no situation, water has no clip, and Airborne is not the word Air"
    );
    assert_eq!(
        transition_clips(&bin, 77, &cac, &tokens).expect("other graph"),
        vec![None; 6]
    );
}
