use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use dekan_wad::prop::tree::{self, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};

use super::sound_bank::{self, Object, wwise_id};
use crate::binary::u32_le;
use crate::error::ClassicError;
use crate::gear_toggle::{FIELD_HASH, bin_error, h, hash_value, named, pointer};
use crate::vfx_markers::{raw_text, text};

const SKIN_CLASS: &str = "SkinCharacterDataProperties";
const KNOWN_VERSION: u32 = 145;
const ACTION: u8 = 3;
const EVENT: u8 = 4;
const SWITCH_CONTAINER: u8 = 6;
const SET_SWITCH: u16 = 0x1901;
const SWITCH_GROUP: u8 = 0;
const MAX_ACTIONS: usize = 127;
const SOUND_EVENT: &str = "DekanFormSound";
const FIELD_BOOL: u8 = 1;
const FIELD_POINTER: u8 = 0x82;

pub struct FormSounds {
    pub bank: Vec<u8>,
    pub events: Vec<String>,
}

fn gear_value(form: usize) -> u32 {
    wwise_id(&format!("gear_{form}"))
}

#[must_use]
pub fn gear_groups(objects: &[Object], forms: usize) -> BTreeSet<u32> {
    let values: BTreeSet<u32> = (0..forms).map(gear_value).collect();
    objects
        .iter()
        .filter(|o| o.kind == SWITCH_CONTAINER)
        .filter_map(|o| {
            let first = (5..o.payload.len())
                .find(|at| u32_le(&o.payload, *at).is_some_and(|v| values.contains(&v)))?;
            if o.payload[first - 5] == SWITCH_GROUP {
                u32_le(&o.payload, first - 4)
            } else {
                None
            }
        })
        .collect()
}

fn set_switch(group: u32, value: u32) -> Vec<u8> {
    let mut payload = SET_SWITCH.to_le_bytes().to_vec();
    payload.extend_from_slice(&value.to_le_bytes());
    payload.extend_from_slice(&[0, 0, 0]);
    payload.extend_from_slice(&group.to_le_bytes());
    payload.extend_from_slice(&value.to_le_bytes());
    payload
}

pub fn add_form_switches(bank: &[u8], forms: usize) -> Result<Option<FormSounds>, ClassicError> {
    let mut parsed = sound_bank::parse(bank)?;
    if parsed.version() != Some(KNOWN_VERSION) {
        return Ok(None);
    }
    let objects = parsed.objects()?;
    let groups = gear_groups(&objects, forms);
    if groups.is_empty() {
        return Ok(None);
    }
    if groups.len() > MAX_ACTIONS {
        return Err(ClassicError::Audio(format!(
            "{} switch groups",
            groups.len()
        )));
    }
    let mut taken: BTreeSet<u32> = objects.iter().map(|o| o.id).collect();
    let mut claim = |id: u32, name: &str| {
        if taken.insert(id) {
            Ok(id)
        } else {
            Err(ClassicError::Audio(format!(
                "'{name}' collides with an id the bank has"
            )))
        }
    };
    let mut added = Vec::new();
    let mut events = Vec::with_capacity(forms);
    for form in 0..forms {
        let event = format!("Dekan_Form{form}_Sound");
        let mut actions = Vec::with_capacity(groups.len());
        for group in &groups {
            let name = format!("{event}_{group:08x}");
            let id = claim(wwise_id(&name), &name)?;
            added.push(Object {
                kind: ACTION,
                id,
                payload: set_switch(*group, gear_value(form)),
            });
            actions.push(id);
        }
        let mut payload = vec![groups.len() as u8];
        actions
            .iter()
            .for_each(|id| payload.extend_from_slice(&id.to_le_bytes()));
        added.push(Object {
            kind: EVENT,
            id: claim(wwise_id(&event), &event)?,
            payload,
        });
        events.push(event);
    }
    parsed.append(&added)?;
    Ok(Some(FormSounds {
        bank: parsed.write()?,
        events,
    }))
}

fn audio_units(fields: &mut [tree::Field]) -> Option<&mut Vec<Value>> {
    tree::field_mut(fields, h("skinAudioProperties"))
        .and_then(Value::fields_mut)
        .and_then(|audio| tree::field_mut(audio, h("bankUnits")))
        .and_then(Value::items_mut)
}

fn bank_list(unit: &[tree::Field]) -> Vec<String> {
    tree::field(unit, h("bankPath"))
        .and_then(Value::items)
        .unwrap_or_default()
        .iter()
        .filter_map(raw_text)
        .collect()
}

pub fn bank_paths(skin_bin: &[u8]) -> Result<Vec<String>, ClassicError> {
    let file = parse_prop_file(skin_bin).map_err(bin_error)?;
    let mut paths = Vec::new();
    for entry in file
        .entries
        .iter()
        .filter(|e| e.class_hash == h(SKIN_CLASS))
    {
        let fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
        let units = tree::field(&fields, h("skinAudioProperties"))
            .and_then(Value::fields)
            .and_then(|audio| tree::field(audio, h("bankUnits")))
            .and_then(Value::items)
            .unwrap_or_default();
        for unit in units.iter().filter_map(Value::fields) {
            paths.extend(bank_list(unit).into_iter().filter(|p| {
                Path::new(p)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("bnk"))
            }));
        }
    }
    Ok(paths)
}

fn sound_event(name: &str) -> Value {
    let flag = || Value::Raw {
        kind: FIELD_BOOL,
        bytes: vec![0],
    };
    pointer(
        "SoundEventData",
        vec![
            named("mName", hash_value(h(SOUND_EVENT))),
            named("mSoundName", text(name)),
            named("mIsLoop", flag()),
            named("mIsKillEvent", flag()),
        ],
    )
}

pub fn fire_on_entry(
    graph_bin: &[u8],
    graph_key: u32,
    events: &[String],
) -> Result<Vec<u8>, ClassicError> {
    let mut file = parse_prop_file(graph_bin).map_err(bin_error)?;
    let entry = file
        .entries
        .iter_mut()
        .find(|e| e.key_hash == graph_key)
        .ok_or_else(|| ClassicError::Bin("the form graph is missing".into()))?;
    let mut fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
    let Some(Value::Map { entries: clips, .. }) = tree::field_mut(&mut fields, h("mClipDataMap"))
    else {
        return Err(ClassicError::Bin("the form graph has no clips".into()));
    };
    let entry_clips: BTreeMap<u32, &str> = events
        .iter()
        .enumerate()
        .flat_map(|(form, event)| {
            [
                crate::form_state::state_key(form),
                crate::form_state::kick_key(form),
            ]
            .map(|key| (key, event.as_str()))
        })
        .collect();
    let sound_key = hash_value(h(SOUND_EVENT));
    for (key, clip) in clips.iter_mut() {
        let Some(event) = key.as_u32().and_then(|k| entry_clips.get(&k)) else {
            continue;
        };
        let Some(clip_fields) = clip.fields_mut() else {
            continue;
        };
        let sound = (sound_key.clone(), sound_event(event));
        match tree::field_mut(clip_fields, h("mEventDataMap")) {
            Some(Value::Map { entries, .. }) => entries.push(sound),
            _ => tree::set_field(
                clip_fields,
                h("mEventDataMap"),
                Value::Map {
                    key: FIELD_HASH,
                    value: FIELD_POINTER,
                    entries: vec![sound],
                },
            ),
        }
    }
    entry.body = tree::write_fields(&fields).map_err(bin_error)?;
    serialize_prop_file(&file).map_err(bin_error)
}

pub fn list_events(
    skin_bin: &[u8],
    bank_path: &str,
    events: &[String],
) -> Result<Vec<u8>, ClassicError> {
    let mut file = parse_prop_file(skin_bin).map_err(bin_error)?;
    let mut loaded = false;
    for entry in file
        .entries
        .iter_mut()
        .filter(|e| e.class_hash == h(SKIN_CLASS))
    {
        let mut fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
        for unit in audio_units(&mut fields)
            .into_iter()
            .flatten()
            .filter_map(Value::fields_mut)
        {
            if !bank_list(unit)
                .iter()
                .any(|p| p.eq_ignore_ascii_case(bank_path))
            {
                continue;
            }
            loaded = true;
            if let Some(list) = tree::field_mut(unit, h("events")).and_then(Value::items_mut) {
                list.extend(events.iter().map(|e| text(e)));
            }
        }
        entry.body = tree::write_fields(&fields).map_err(bin_error)?;
    }
    if !loaded {
        return Err(ClassicError::Audio(format!(
            "no bank unit of the skin loads '{bank_path}'"
        )));
    }
    serialize_prop_file(&file).map_err(bin_error)
}

#[cfg(test)]
mod tests;
