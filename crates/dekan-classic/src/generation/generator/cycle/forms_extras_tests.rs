use super::forms_tests::{BANK, SKIN, elemental_build};
use super::tests::*;
use super::*;
use crate::form_state::{kick_key, state_key};
use crate::gear_toggle::h;
use dekan_wad::prop::tree::{self, Value};

fn sounds_of(clip: &Value) -> Vec<String> {
    let Some(Value::Map { entries, .. }) = clip
        .fields()
        .and_then(|f| tree::field(f, h("mEventDataMap")))
    else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(_, event)| event.fields().and_then(|f| tree::field(f, h("mSoundName"))))
        .filter_map(|name| match name {
            Value::Raw { bytes, .. } => Some(String::from_utf8_lossy(&bytes[2..]).into_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn ctrl5_plays_the_forms_own_transformation_and_switches_the_skins_sounds_to_that_form() {
    let (mods_dir, folder) = elemental_build("transition_sound");
    let root = mods_dir.join(&folder).join("WAD").join("Zed.wad.client");

    let graph = std::fs::read(root.join(format!("data/characters/zed/animations/skin{SKIN}.bin")))
        .expect("form graph");
    let graph = parse_prop_file(&graph).expect("graph");
    let fields = tree::parse_fields(&graph.entries[0].body).expect("fields");
    let Some(Value::Map { entries: clips, .. }) = tree::field(&fields, h("mClipDataMap")) else {
        panic!("clips");
    };
    let clip = |key: u32| {
        &clips
            .iter()
            .find(|(k, _)| k.as_u32() == Some(key))
            .expect("clip")
            .1
    };
    let entering_storm = sounds_of(clip(kick_key(1)));
    assert!(
        entering_storm.contains(&"Play_sfx_Zed_Transform_Storm_buffactivate".to_owned()),
        "the storm form plays its own transformation: {entering_storm:?}"
    );
    assert!(entering_storm.contains(&"Dekan_Form1_Sound".to_owned()));
    assert!(sounds_of(clip(state_key(0))).contains(&"Dekan_Form0_Sound".to_owned()));

    let bank = std::fs::read(root.join(BANK.to_ascii_lowercase())).expect("bank");
    let objects = crate::sound_bank::parse(&bank)
        .expect("bank")
        .objects()
        .expect("objects");
    for form in 0..2 {
        let id = crate::sound_bank::wwise_id(&format!("Dekan_Form{form}_Sound"));
        assert!(
            objects.iter().any(|o| o.kind == 4 && o.id == id),
            "form {form} event"
        );
    }

    let skin0 = generated_skin0(&mods_dir, &folder, "zed").expect("skin0");
    let skin = skin0
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
        .expect("skin");
    let skin = tree::parse_fields(&skin.body).expect("skin fields");
    let listed = tree::field(&skin, h("skinAudioProperties"))
        .and_then(Value::fields)
        .and_then(|a| tree::field(a, h("bankUnits")))
        .and_then(Value::items)
        .and_then(|units| units[0].fields())
        .and_then(|unit| tree::field(unit, h("events")))
        .and_then(Value::items)
        .map_or(0, <[Value]>::len);
    assert_eq!(listed, 3, "the bank's own event plus one per form");
}
