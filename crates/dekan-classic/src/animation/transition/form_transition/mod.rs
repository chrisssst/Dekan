use std::collections::BTreeSet;

use dekan_wad::prop::parse_prop_file;
use dekan_wad::prop::tree::{self, Value};

use crate::error::ClassicError;
use crate::gear_toggle::{bin_error, h};
use crate::vfx_markers::{raw_text, walk};

#[must_use]
pub fn form_token(champion: &str, model_path: &str) -> Option<String> {
    let lower = model_path.to_ascii_lowercase();
    let start = lower.find("characters/")? + "characters/".len();
    let character = &model_path[start..start + lower[start..].find('/')?];
    let rest = character
        .get(..champion.len())
        .filter(|prefix| prefix.eq_ignore_ascii_case(champion))
        .map(|_| &character[champion.len()..])?;
    (!rest.is_empty()).then(|| rest.to_ascii_lowercase())
}

pub fn situation_keys(skin_bin: &[u8]) -> Result<BTreeSet<u32>, ClassicError> {
    let file = parse_prop_file(skin_bin).map_err(bin_error)?;
    let mut keys = BTreeSet::new();
    for entry in file
        .entries
        .iter()
        .filter(|e| e.class_hash == h("ContextualActionData"))
    {
        let fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
        if let Some(Value::Map { entries, .. }) = tree::field(&fields, h("mSituations")) {
            keys.extend(entries.iter().filter_map(|(k, _)| k.as_u32()));
        }
    }
    Ok(keys)
}

fn situation_sounds(
    graph_bin: &[u8],
    graph_key: u32,
    situations: &BTreeSet<u32>,
) -> Result<Vec<(u32, Vec<String>)>, ClassicError> {
    let file = parse_prop_file(graph_bin).map_err(bin_error)?;
    let Some(entry) = file.entries.iter().find(|e| e.key_hash == graph_key) else {
        return Ok(Vec::new());
    };
    let fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
    let Some(Value::Map { entries, .. }) = tree::field(&fields, h("mClipDataMap")) else {
        return Ok(Vec::new());
    };
    let sound_name = h("mSoundName");
    Ok(entries
        .iter()
        .filter_map(|(key, clip)| {
            let key = key.as_u32().filter(|k| situations.contains(k))?;
            let mut sounds = Vec::new();
            walk(clip, &mut |value| {
                if let Value::Struct { fields, .. } = value {
                    sounds.extend(tree::field(fields, sound_name).and_then(raw_text));
                }
            });
            Some((key, sounds))
        })
        .collect())
}

pub fn transition_clips(
    graph_bin: &[u8],
    graph_key: u32,
    situations: &BTreeSet<u32>,
    tokens: &[Option<String>],
) -> Result<Vec<Option<u32>>, ClassicError> {
    let clips = situation_sounds(graph_bin, graph_key, situations)?;
    Ok(tokens
        .iter()
        .map(|token| {
            let token = token.as_deref()?;
            let mut matches = clips.iter().filter(|(_, names)| {
                names
                    .iter()
                    .any(|name| name.split('_').any(|word| word.eq_ignore_ascii_case(token)))
            });
            let (first, _) = matches.next()?;
            matches.next().is_none().then_some(*first)
        })
        .collect())
}

#[cfg(test)]
mod tests;
