use std::collections::BTreeSet;

use dekan_wad::prop::tree::{self, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};

use crate::error::ClassicError;
use crate::gear_toggle::{ClipEntries, FIELD_HASH, bin_error, clip_at, clip_refs, h, hash_value};

const DEFAULT_TICK: f32 = 1.0 / 30.0;
const MAX_SOUND_DEPTH: usize = 16;
const FIELD_STRING: u8 = 16;

pub type AliasedGraph = (Vec<u8>, Vec<ClipAlias>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipAlias {
    pub missing: u32,
    pub variant: u32,
    pub variants: usize,
}

fn clip_entries(fields: &mut [tree::Field]) -> Option<&mut ClipEntries> {
    match tree::field_mut(fields, h("mClipDataMap")) {
        Some(Value::Map {
            key: FIELD_HASH,
            entries,
            ..
        }) => Some(entries),
        _ => None,
    }
}

fn clip_keys(graph_bin: &[u8], graph_key: u32) -> Result<Option<BTreeSet<u32>>, ClassicError> {
    let file = parse_prop_file(graph_bin).map_err(bin_error)?;
    let Some(entry) = file.entries.iter().find(|e| e.key_hash == graph_key) else {
        return Ok(None);
    };
    let mut fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
    Ok(clip_entries(&mut fields)
        .map(|entries| entries.iter().filter_map(|(k, _)| k.as_u32()).collect()))
}

fn track_of(clip: &Value) -> Option<u32> {
    clip.fields()
        .and_then(|f| tree::field(f, h("mTrackDataName")))
        .and_then(Value::as_u32)
}

fn tick_of(clip: &Value) -> f32 {
    clip.fields()
        .and_then(|f| tree::field(f, h("mTickDuration")))
        .and_then(|v| match v {
            Value::Raw { bytes, .. } if bytes.len() == 4 => {
                Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            _ => None,
        })
        .unwrap_or(DEFAULT_TICK)
}

fn children(clip: &Value) -> Vec<u32> {
    let mut out = Vec::new();
    clip_refs(clip, &mut out);
    out
}

fn variant_tick(entries: &ClipEntries, parallel: &Value, missing: u32) -> Option<f32> {
    let on_track: Vec<f32> = children(parallel)
        .into_iter()
        .filter_map(|child| clip_at(entries, child).map(|at| &entries[at].1))
        .filter(|child| {
            child.class() == Some(h("AtomicClipData")) && track_of(child) == Some(missing)
        })
        .map(tick_of)
        .collect();
    match on_track.as_slice() {
        [tick] => Some(*tick),
        _ => None,
    }
}

fn sounds_of(entries: &ClipEntries, key: u32, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_SOUND_DEPTH {
        return;
    }
    let Some(at) = clip_at(entries, key) else {
        return;
    };
    let clip = &entries[at].1;
    if let Some(Value::Map {
        entries: events, ..
    }) = clip
        .fields()
        .and_then(|f| tree::field(f, h("mEventDataMap")))
    {
        for (_, event) in events {
            let sound = event
                .fields()
                .and_then(|f| tree::field(f, h("mSoundName")))
                .and_then(|v| match v {
                    Value::Raw { kind, bytes } if *kind == FIELD_STRING && bytes.len() > 2 => {
                        Some(String::from_utf8_lossy(&bytes[2..]).to_ascii_lowercase())
                    }
                    _ => None,
                });
            out.extend(sound);
        }
    }
    for child in children(clip) {
        sounds_of(entries, child, depth + 1, out);
    }
}

fn plays_spell(entries: &ClipEntries, key: u32, spell: &str) -> bool {
    let mut sounds = Vec::new();
    sounds_of(entries, key, 0, &mut sounds);
    let spell = spell.to_ascii_lowercase();
    sounds.iter().any(|sound| sound.contains(&spell))
}

fn pick_variant(entries: &ClipEntries, missing: u32, spell: &str) -> Option<ClipAlias> {
    let referenced: BTreeSet<u32> = entries
        .iter()
        .flat_map(|(_, clip)| children(clip))
        .collect();
    let variants: Vec<(u32, f32)> = entries
        .iter()
        .filter_map(|(key, clip)| {
            let key = key.as_u32()?;
            if referenced.contains(&key) || clip.class() != Some(h("ParallelClipData")) {
                return None;
            }
            variant_tick(entries, clip, missing).map(|tick| (key, tick))
        })
        .collect();
    if variants.len() < 2
        || !variants
            .iter()
            .all(|(key, _)| plays_spell(entries, *key, spell))
    {
        return None;
    }
    let (variant, _) = variants
        .iter()
        .copied()
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))?;
    Some(ClipAlias {
        missing,
        variant,
        variants: variants.len(),
    })
}

#[must_use]
pub fn spell_slot_clip(slot: usize) -> u32 {
    h(&format!("Spell{}", slot + 1))
}

pub fn alias_missing_clips(
    graph_bin: &[u8],
    graph_key: u32,
    base_graph_bin: &[u8],
    base_graph_key: u32,
    spells: &[String],
) -> Result<Option<AliasedGraph>, ClassicError> {
    let Some(base_keys) = clip_keys(base_graph_bin, base_graph_key)? else {
        return Ok(None);
    };
    let mut file = parse_prop_file(graph_bin).map_err(bin_error)?;
    let Some(at) = file.entries.iter().position(|e| e.key_hash == graph_key) else {
        return Ok(None);
    };
    let mut graph = tree::parse_fields(&file.entries[at].body).map_err(bin_error)?;
    let Some(entries) = clip_entries(&mut graph) else {
        return Ok(None);
    };
    let present: BTreeSet<u32> = entries.iter().filter_map(|(k, _)| k.as_u32()).collect();
    let aliases: Vec<ClipAlias> = spells
        .iter()
        .enumerate()
        .map(|(slot, spell)| (spell_slot_clip(slot), spell))
        .filter(|(clip, _)| base_keys.contains(clip) && !present.contains(clip))
        .filter_map(|(clip, spell)| pick_variant(entries, clip, spell))
        .collect();
    if aliases.is_empty() {
        return Ok(None);
    }
    for alias in &aliases {
        let Some(source) = clip_at(entries, alias.variant) else {
            continue;
        };
        let clip = entries[source].1.clone();
        entries.push((hash_value(alias.missing), clip));
    }
    file.entries[at].body = tree::write_fields(&graph).map_err(bin_error)?;
    let bytes = serialize_prop_file(&file).map_err(bin_error)?;
    Ok(Some((bytes, aliases)))
}

#[must_use]
pub fn spell_names(record_bin: &[u8]) -> Vec<String> {
    let Ok(file) = parse_prop_file(record_bin) else {
        return Vec::new();
    };
    file.entries
        .iter()
        .filter(|e| e.class_hash == h("CharacterRecord"))
        .find_map(|e| tree::parse_fields(&e.body).ok())
        .and_then(|fields| {
            tree::field(&fields, h("spellNames"))
                .and_then(Value::items)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| match item {
                            Value::Raw { kind, bytes }
                                if *kind == FIELD_STRING && bytes.len() > 2 =>
                            {
                                let name = String::from_utf8_lossy(&bytes[2..]).into_owned();
                                Some(name.rsplit('/').next().unwrap_or_default().to_owned())
                            }
                            _ => None,
                        })
                        .collect()
                })
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "clip_alias_tests.rs"]
mod tests;
