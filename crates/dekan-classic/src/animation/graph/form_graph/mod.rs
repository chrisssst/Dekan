use std::collections::{BTreeMap, BTreeSet};

use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};
use tracing::{debug, warn};

use crate::error::ClassicError;
use crate::form_clips::{animation_of, specialize_gear_choices, strip_form_parts};
use crate::gear_toggle::{
    FIELD_HASH, GearSwap, add_event, bin_error, clip_refs, condition_on_part, h, hash_list,
    hash_value, named, pointer, rename_refs, visibility_event,
};

const FIELD_U8: u8 = 3;
const FIELD_U32: u8 = 7;
const FIELD_F32: u8 = 10;
const FIELD_LIST: u8 = 0x80;
const FIELD_POINTER: u8 = 0x82;
const FIELD_EMBED: u8 = 0x83;
const FORM_TRACK: &str = "DekanForm";
const NO_POSE_MASK: &str = "DekanNoPose";
const HELD: u32 = 14;
const CARRIER_FALLBACKS: [&str; 3] = ["Idle1", "Idle1_Base", "Idle_Base"];
const MAX_CLIPS: usize = 40_000;

fn copy_key(form: usize, key: u32) -> u32 {
    h(&format!("DekanForm{form}_{key:08x}"))
}

fn route_key(form: usize, key: u32) -> u32 {
    h(&format!("DekanRoute{form}_{key:08x}"))
}

fn toggle_key(form: usize) -> u32 {
    if form == 1 {
        h("Toggle")
    } else {
        h(&format!("DekanToggle{form}"))
    }
}

fn map_mut<'a>(graph: &'a mut [Field], name: &str) -> Option<&'a mut Vec<(Value, Value)>> {
    match tree::field_mut(graph, h(name)) {
        Some(Value::Map {
            key: FIELD_HASH,
            entries,
            ..
        }) => Some(entries),
        _ => None,
    }
}

fn keys_of(graph: &mut [Field], name: &str) -> BTreeSet<u32> {
    map_mut(graph, name)
        .map(|entries| entries.iter().filter_map(|(k, _)| k.as_u32()).collect())
        .unwrap_or_default()
}

fn joint_count(graph: &mut [Field]) -> Option<usize> {
    map_mut(graph, "mMaskDataMap")?
        .iter()
        .find_map(|(_, mask)| {
            tree::field(mask.fields()?, h("mWeightList"))
                .and_then(Value::items)
                .map(<[Value]>::len)
                .filter(|len| *len > 0)
        })
}

fn fit_masks(graph: &mut Vec<Field>, joints: usize) {
    match map_mut(graph, "mMaskDataMap") {
        Some(masks) => {
            for (_, mask) in masks.iter_mut() {
                if let Some(weights) = mask
                    .fields_mut()
                    .and_then(|f| tree::field_mut(f, h("mWeightList")))
                    .and_then(Value::items_mut)
                {
                    weights.resize(joints, raw(FIELD_F32, 0f32.to_le_bytes().to_vec()));
                }
            }
        }
        None => tree::set_field(
            graph,
            h("mMaskDataMap"),
            Value::Map {
                key: FIELD_HASH,
                value: FIELD_EMBED,
                entries: Vec::new(),
            },
        ),
    }
}

fn raw(kind: u8, bytes: Vec<u8>) -> Value {
    Value::Raw { kind, bytes }
}

fn form_event(name: &str, show: &[u32], hide: &[u32]) -> Value {
    let mut event = visibility_event(show, hide);
    if let Some(fields) = event.fields_mut() {
        fields.insert(0, named("mName", hash_value(h(name))));
        fields.insert(
            1,
            named("mStartFrame", raw(FIELD_F32, 0f32.to_le_bytes().to_vec())),
        );
    }
    event
}

fn state_clip(event: Value, animation: Value) -> Value {
    pointer(
        "AtomicClipData",
        vec![
            named(
                "mFlags",
                Value::Raw {
                    kind: FIELD_U32,
                    bytes: HELD.to_le_bytes().to_vec(),
                },
            ),
            named("mTrackDataName", hash_value(h(FORM_TRACK))),
            named("mMaskDataName", hash_value(h(NO_POSE_MASK))),
            named(
                "mEventDataMap",
                Value::Map {
                    key: FIELD_HASH,
                    value: FIELD_POINTER,
                    entries: vec![(hash_value(h("DekanGearSwap")), event)],
                },
            ),
            named("mAnimationResourceData", animation),
        ],
    )
}

struct Forms {
    own: Vec<Vec<u32>>,
    others: Vec<Vec<u32>>,
    parts: BTreeSet<u32>,
}

fn forms(swaps: &[GearSwap]) -> Forms {
    let parts: BTreeSet<u32> = swaps.iter().flat_map(|s| s.show.iter().copied()).collect();
    let others = swaps
        .iter()
        .map(|swap| {
            parts
                .iter()
                .chain(swap.hide.iter())
                .copied()
                .filter(|part| !swap.show.contains(part))
                .collect::<BTreeSet<u32>>()
                .into_iter()
                .collect()
        })
        .collect();
    Forms {
        own: swaps.iter().map(|s| s.show.clone()).collect(),
        others,
        parts,
    }
}

fn every_reference_resolves(graph: &mut [Field]) -> bool {
    let tracks = keys_of(graph, "mTrackDataMap");
    let masks = keys_of(graph, "mMaskDataMap");
    let Some(clips) = map_mut(graph, "mClipDataMap") else {
        return false;
    };
    let keys: BTreeSet<u32> = clips.iter().filter_map(|(k, _)| k.as_u32()).collect();
    keys.len() == clips.len()
        && clips.iter().all(|(_, clip)| {
            let mut refs = Vec::new();
            clip_refs(clip, &mut refs);
            let fields = clip.fields().unwrap_or_default();
            let named_in = |name: &str, set: &BTreeSet<u32>| {
                tree::field(fields, h(name))
                    .and_then(Value::as_u32)
                    .is_none_or(|key| set.contains(&key))
            };
            refs.iter().all(|r| keys.contains(r))
                && named_in("mTrackDataName", &tracks)
                && named_in("mMaskDataName", &masks)
        })
}

pub fn build_form_graph(
    graph_bin: &[u8],
    graph_key: u32,
    swaps: &[GearSwap],
    markers: &[u32],
    skeleton_joints: Option<usize>,
) -> Result<Option<(Vec<u8>, usize)>, ClassicError> {
    let count = swaps.len();
    if count < 2 || markers.len() + 1 != count {
        return Ok(None);
    }
    let mut file = parse_prop_file(graph_bin).map_err(bin_error)?;
    let Some(at) = file.entries.iter().position(|e| e.key_hash == graph_key) else {
        return Ok(None);
    };
    let mut graph = tree::parse_fields(&file.entries[at].body).map_err(bin_error)?;
    let from_masks = joint_count(&mut graph);
    let Some(joints) = from_masks.max(skeleton_joints) else {
        return Ok(None);
    };
    if joints > from_masks.unwrap_or(0) {
        fit_masks(&mut graph, joints);
    }
    if map_mut(&mut graph, "mTrackDataMap").is_none()
        || keys_of(&mut graph, "mTrackDataMap").contains(&h(FORM_TRACK))
    {
        debug!(
            graph = format!("{graph_key:08x}"),
            "Form graph skipped: no track map, or already carries the form track"
        );
        return Ok(None);
    }
    let Some(clips) = map_mut(&mut graph, "mClipDataMap") else {
        return Ok(None);
    };
    let originals: BTreeMap<u32, Value> = clips
        .iter()
        .filter_map(|(k, v)| Some((k.as_u32()?, v.clone())))
        .collect();
    if originals.len() * (2 * count - 1) + 4 * count > MAX_CLIPS {
        warn!(
            graph = format!("{graph_key:08x}"),
            clips = originals.len(),
            forms = count,
            "Form graph skipped: the copies of every clip would exceed the clip limit"
        );
        return Ok(None);
    }
    if originals.len() != clips.len() || originals.contains_key(&h("Toggle")) {
        debug!(
            graph = format!("{graph_key:08x}"),
            "Form graph skipped: duplicate clip keys or a Toggle clip already exists"
        );
        return Ok(None);
    }

    let forms = forms(swaps);
    let mut entries: Vec<(Value, Value)> =
        Vec::with_capacity(originals.len() * (2 * count - 1) + 4 * count);
    for form in 0..count {
        let renames: BTreeMap<u32, u32> =
            originals.keys().map(|k| (*k, copy_key(form, *k))).collect();
        let event = form_event("DekanHideOthers", &forms.own[form], &forms.others[form]);
        for (key, clip) in &originals {
            let mut copy = clip.clone();
            rename_refs(&mut copy, &renames);
            if copy.class() == Some(h("AtomicClipData")) {
                strip_form_parts(&mut copy, &forms.parts);
                add_event(&mut copy, &event)?;
            }
            entries.push((hash_value(copy_key(form, *key)), copy));
        }
    }
    specialize_gear_choices(&mut entries, &originals, count, copy_key);
    for key in originals.keys() {
        let mut next = copy_key(0, *key);
        for form in (1..count).rev() {
            let at_key = if form == 1 {
                *key
            } else {
                route_key(form, *key)
            };
            entries.push((
                hash_value(at_key),
                condition_on_part(markers[form - 1], copy_key(form, *key), next),
            ));
            next = at_key;
        }
    }

    for (form, swap) in swaps.iter().enumerate() {
        let carrier = swap
            .equip
            .as_deref()
            .map(h)
            .into_iter()
            .chain(swap.transition)
            .chain(CARRIER_FALLBACKS.iter().map(|name| h(name)))
            .find(|key| originals.contains_key(key));
        let Some(carrier) = carrier else {
            return Ok(None);
        };
        let Some(animation) = animation_of(&originals, carrier, 0) else {
            return Ok(None);
        };
        let show: Vec<u32> = forms.own[form]
            .iter()
            .copied()
            .chain(form.checked_sub(1).map(|m| markers[m]))
            .collect();
        let hide: Vec<u32> = forms.others[form]
            .iter()
            .copied()
            .chain(
                markers
                    .iter()
                    .enumerate()
                    .filter(|(m, _)| m + 1 != form)
                    .map(|(_, marker)| *marker),
            )
            .collect();
        let event = form_event("DekanSwap", &show, &hide);
        let state = crate::form_state::state_key(form);
        entries.push((hash_value(state), state_clip(event.clone(), animation)));
        let kick = if originals.get(&carrier).and_then(Value::class) == Some(h("AtomicClipData")) {
            let kick = crate::form_state::kick_key(form);
            let mut clip = entries
                .iter()
                .find(|(k, _)| k.as_u32() == Some(copy_key(form, carrier)))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| ClassicError::Bin("a form copy is missing".into()))?;
            add_event(&mut clip, &event)?;
            entries.push((hash_value(kick), clip));
            kick
        } else {
            copy_key(form, carrier)
        };
        entries.push((
            hash_value(crate::form_state::swap_key(form)),
            pointer(
                "ParallelClipData",
                vec![
                    named("mFlags", raw(FIELD_U32, HELD.to_le_bytes().to_vec())),
                    named("mClipNameList", hash_list(&[state, kick])),
                ],
            ),
        ));
    }
    let swap_key = crate::form_state::swap_key;
    let mut next = swap_key(1);
    for form in (1..count).rev() {
        entries.push((
            hash_value(toggle_key(form)),
            condition_on_part(markers[form - 1], swap_key((form + 1) % count), next),
        ));
        next = toggle_key(form);
    }
    *clips = entries;

    if let Some(tracks) = map_mut(&mut graph, "mTrackDataMap") {
        tracks.push((
            hash_value(h(FORM_TRACK)),
            Value::Struct {
                kind: FIELD_EMBED,
                class: h("TrackData"),
                fields: vec![
                    named("mPriority", raw(FIELD_U8, vec![1])),
                    named("mBlendWeight", raw(FIELD_F32, 0f32.to_le_bytes().to_vec())),
                ],
            },
        ));
    }
    if let Some(masks) = map_mut(&mut graph, "mMaskDataMap") {
        let id = masks
            .iter()
            .filter_map(|(_, mask)| tree::field(mask.fields()?, h("mId"))?.as_u32())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let weights = Value::List {
            kind: FIELD_LIST,
            element: FIELD_F32,
            items: vec![raw(FIELD_F32, 0f32.to_le_bytes().to_vec()); joints],
        };
        masks.push((
            hash_value(h(NO_POSE_MASK)),
            Value::Struct {
                kind: FIELD_EMBED,
                class: h("MaskData"),
                fields: vec![
                    named("mId", raw(FIELD_U32, id.to_le_bytes().to_vec())),
                    named("mWeightList", weights),
                ],
            },
        ));
    }
    if !every_reference_resolves(&mut graph) {
        return Ok(None);
    }
    file.entries[at].body = tree::write_fields(&graph).map_err(bin_error)?;
    serialize_prop_file(&file)
        .map(|bytes| Some((bytes, originals.len())))
        .map_err(bin_error)
}

#[cfg(test)]
mod tests;
