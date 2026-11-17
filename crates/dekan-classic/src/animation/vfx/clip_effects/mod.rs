use std::collections::{BTreeMap, BTreeSet};

use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};

use crate::error::ClassicError;
use crate::form_graph::copy_key;
use crate::form_vfx::FormKeys;
use crate::gear_toggle::{bin_error, h, hash_value};

fn effect_key(event: &[Field]) -> Option<u32> {
    tree::field(event, h("mEffectKey")).and_then(Value::as_u32)
}

fn kills(event: &[Field]) -> bool {
    matches!(
        tree::field(event, h("mIsKillEvent")),
        Some(Value::Raw { bytes, .. }) if bytes.first().is_some_and(|b| *b != 0)
    )
}

fn is_particle_event(event: &Value) -> bool {
    event.class() == Some(h("ParticleEventData"))
}

fn particle_events(clip: &Value) -> impl Iterator<Item = &[Field]> {
    let events = match clip
        .fields()
        .and_then(|f| tree::field(f, h("mEventDataMap")))
    {
        Some(Value::Map { entries, .. }) => entries.as_slice(),
        _ => &[],
    };
    events
        .iter()
        .filter(|(_, event)| is_particle_event(event))
        .filter_map(|(_, event)| event.fields())
}

pub fn fire_own_effects(
    graph_bin: &[u8],
    graph_key: u32,
    form_keys: &FormKeys,
) -> Result<Option<(Vec<u8>, usize)>, ClassicError> {
    let forms = form_keys.values().map(Vec::len).max().unwrap_or(0);
    if forms == 0 {
        return Ok(None);
    }
    let mut file = parse_prop_file(graph_bin).map_err(bin_error)?;
    let Some(at) = file.entries.iter().position(|e| e.key_hash == graph_key) else {
        return Ok(None);
    };
    let mut graph = tree::parse_fields(&file.entries[at].body).map_err(bin_error)?;
    let Some(Value::Map { entries: clips, .. }) = tree::field_mut(&mut graph, h("mClipDataMap"))
    else {
        return Ok(None);
    };
    let killed: BTreeSet<u32> = clips
        .iter()
        .flat_map(|(_, clip)| particle_events(clip))
        .filter(|event| kills(event))
        .filter_map(effect_key)
        .collect();
    let keys: BTreeSet<u32> = clips.iter().filter_map(|(k, _)| k.as_u32()).collect();
    let form_of: BTreeMap<u32, usize> = keys
        .iter()
        .flat_map(|key| (0..forms).map(move |form| (copy_key(form, *key), form)))
        .filter(|(copy, _)| keys.contains(copy))
        .collect();
    let mut fired = 0;
    for (key, clip) in clips.iter_mut() {
        let Some(form) = key.as_u32().and_then(|k| form_of.get(&k)).copied() else {
            continue;
        };
        let Some(Value::Map {
            entries: events, ..
        }) = clip
            .fields_mut()
            .and_then(|f| tree::field_mut(f, h("mEventDataMap")))
        else {
            continue;
        };
        for (_, event) in events.iter_mut().filter(|(_, e)| is_particle_event(e)) {
            let Some(fields) = event.fields_mut() else {
                continue;
            };
            let own = effect_key(fields)
                .filter(|k| !killed.contains(k))
                .and_then(|k| form_keys.get(&k))
                .and_then(|per_form| per_form.get(form).copied().flatten());
            if let Some(own) = own {
                tree::set_field(fields, h("mEffectKey"), hash_value(own));
                fired += 1;
            }
        }
    }
    if fired == 0 {
        return Ok(None);
    }
    file.entries[at].body = tree::write_fields(&graph).map_err(bin_error)?;
    serialize_prop_file(&file)
        .map(|bytes| Some((bytes, fired)))
        .map_err(bin_error)
}

#[cfg(test)]
mod tests;
