use std::collections::{BTreeMap, BTreeSet};

use dekan_wad::prop::tree::{self, Value};

use crate::gear_toggle::{clip_refs, h};

const MAX_DEPTH: usize = 64;

fn without_parts(list: Option<&mut Value>, parts: &BTreeSet<u32>) -> (bool, bool) {
    let Some(items) = list.and_then(Value::items_mut) else {
        return (false, true);
    };
    let before = items.len();
    items.retain(|item| item.as_u32().is_none_or(|part| !parts.contains(&part)));
    (items.len() != before, items.is_empty())
}

pub(crate) fn strip_form_parts(clip: &mut Value, parts: &BTreeSet<u32>) {
    let Some(Value::Map { entries, .. }) = clip
        .fields_mut()
        .and_then(|fields| tree::field_mut(fields, h("mEventDataMap")))
    else {
        return;
    };
    entries.retain_mut(|(_, event)| {
        if event.class() != Some(h("SubmeshVisibilityEventData")) {
            return true;
        }
        let Some(fields) = event.fields_mut() else {
            return true;
        };
        let (show_changed, no_show) =
            without_parts(tree::field_mut(fields, h("mShowSubmeshList")), parts);
        let (hide_changed, no_hide) =
            without_parts(tree::field_mut(fields, h("mHideSubmeshList")), parts);
        !((show_changed || hide_changed) && no_show && no_hide)
    });
}

pub(crate) fn animation_of(clips: &BTreeMap<u32, Value>, key: u32, depth: usize) -> Option<Value> {
    let clip = clips.get(&key)?;
    if clip.class() == Some(h("AtomicClipData")) {
        return tree::field(clip.fields()?, h("mAnimationResourceData")).cloned();
    }
    if depth >= MAX_DEPTH {
        return None;
    }
    let mut refs = Vec::new();
    clip_refs(clip, &mut refs);
    refs.into_iter()
        .find_map(|child| animation_of(clips, child, depth + 1))
}

fn gear_choice(clip: &Value, form: usize) -> Option<u32> {
    if clip.class() != Some(h("ConditionFloatClipData")) {
        return None;
    }
    let fields = clip.fields()?;
    if tree::field(fields, h("Updater")).and_then(Value::class)
        != Some(h("EquippedGearParametricUpdater"))
    {
        return None;
    }
    let pairs = tree::field(fields, h("mConditionFloatPairDataList")).and_then(Value::items)?;
    let value_of = |pair: &Value| match pair.fields().and_then(|f| tree::field(f, h("mValue"))) {
        Some(Value::Raw { bytes, .. }) => {
            <[u8; 4]>::try_from(bytes.as_slice()).map_or(0.0, f32::from_le_bytes)
        }
        _ => 0.0,
    };
    let gear = form as f32;
    let chosen = pairs
        .iter()
        .rev()
        .find(|pair| value_of(pair) <= gear)
        .or_else(|| pairs.first())?;
    tree::field(chosen.fields()?, h("mClipName")).and_then(Value::as_u32)
}

pub(crate) fn specialize_gear_choices(
    entries: &mut [(Value, Value)],
    originals: &BTreeMap<u32, Value>,
    forms: usize,
    copy_key: impl Fn(usize, u32) -> u32,
) {
    let at = |entries: &[(Value, Value)], key: u32| {
        entries.iter().position(|(k, _)| k.as_u32() == Some(key))
    };
    for form in 0..forms {
        for (key, clip) in originals {
            let Some(child) = gear_choice(clip, form).filter(|c| originals.contains_key(c)) else {
                continue;
            };
            if let (Some(from), Some(to)) = (
                at(entries, copy_key(form, child)),
                at(entries, copy_key(form, *key)),
            ) {
                entries[to].1 = entries[from].1.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests;
