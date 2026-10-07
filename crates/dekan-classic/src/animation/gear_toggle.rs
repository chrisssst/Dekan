use std::collections::{BTreeMap, BTreeSet};

use dekan_wad::hash::prop_key_hash;
use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};

use crate::error::ClassicError;

const FIELD_BOOL: u8 = 1;
const FIELD_STRING: u8 = 16;
pub(crate) const FIELD_HASH: u8 = 17;
const FIELD_LIST: u8 = 0x80;
const FIELD_POINTER: u8 = 0x82;
const MAX_CLIP_DEPTH: usize = 64;
const TOGGLE_CLIP: &str = "Toggle";
const CARRIER_FALLBACKS: [&str; 3] = ["Idle1", "Idle1_Base", "Idle_Base"];
const SINGLE_CLIP_FIELDS: [&str; 3] = [
    "mTrueConditionClipName",
    "mFalseConditionClipName",
    "mClipName",
];
const CLIP_LIST_FIELD: &str = "mClipNameList";

pub(crate) fn h(name: &str) -> u32 {
    prop_key_hash(name)
}

pub(crate) fn bin_error(e: impl std::fmt::Display) -> ClassicError {
    ClassicError::Bin(e.to_string())
}

pub(crate) fn hash_value(hash: u32) -> Value {
    Value::Raw {
        kind: FIELD_HASH,
        bytes: hash.to_le_bytes().to_vec(),
    }
}

fn hash_list(hashes: &[u32]) -> Value {
    Value::List {
        kind: FIELD_LIST,
        element: FIELD_HASH,
        items: hashes.iter().copied().map(hash_value).collect(),
    }
}

fn pointer(class: &str, fields: Vec<Field>) -> Value {
    Value::Struct {
        kind: FIELD_POINTER,
        class: h(class),
        fields,
    }
}

fn named(name: &str, value: Value) -> Field {
    Field {
        name: h(name),
        value,
    }
}

fn hashes_in(fields: &[Field], name: &str) -> Vec<u32> {
    tree::field(fields, h(name))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GearSwap {
    pub show: Vec<u32>,
    pub hide: Vec<u32>,
    pub equip: Option<String>,
}

pub fn gear_swap(gear_body: &[u8]) -> Result<GearSwap, ClassicError> {
    let gear = tree::parse_fields(gear_body).map_err(bin_error)?;
    let data = tree::field(&gear, h("mGearData"))
        .and_then(Value::fields)
        .unwrap_or_default();
    let equip = tree::field(data, h("mEquipAnimation")).and_then(|v| match v {
        Value::Raw { kind, bytes } if *kind == FIELD_STRING && bytes.len() > 2 => {
            Some(String::from_utf8_lossy(&bytes[2..]).into_owned())
        }
        _ => None,
    });
    Ok(GearSwap {
        show: hashes_in(data, "mCharacterSubmeshesToShow"),
        hide: hashes_in(data, "mCharacterSubmeshesToHide"),
        equip,
    })
}

#[must_use]
pub fn markers(swaps: &[GearSwap]) -> Option<Vec<u32>> {
    if swaps.len() < 2 {
        return None;
    }
    swaps
        .iter()
        .enumerate()
        .map(|(i, swap)| {
            swap.show.iter().copied().find(|part| {
                swaps
                    .iter()
                    .enumerate()
                    .all(|(j, other)| j == i || !other.show.contains(part))
            })
        })
        .collect()
}

fn visibility_event(show: &[u32], hide: &[u32]) -> Value {
    pointer(
        "SubmeshVisibilityEventData",
        vec![
            named("mShowSubmeshList", hash_list(show)),
            named("mHideSubmeshList", hash_list(hide)),
        ],
    )
}

fn condition_on_part(part: u32, visible: u32, otherwise: u32) -> Value {
    let driver = part_visible(FIELD_POINTER, part);
    pointer(
        "ConditionBoolClipData",
        vec![
            named(
                "Updater",
                pointer(
                    "LogicDriverBoolParametricUpdater",
                    vec![named("driver", driver)],
                ),
            ),
            named("mTrueConditionClipName", hash_value(visible)),
            named("mFalseConditionClipName", hash_value(otherwise)),
        ],
    )
}

pub(crate) type ClipEntries = Vec<(Value, Value)>;

pub(crate) fn clip_at(entries: &ClipEntries, key: u32) -> Option<usize> {
    entries.iter().position(|(k, _)| k.as_u32() == Some(key))
}

pub(crate) fn clip_refs(value: &Value, out: &mut Vec<u32>) {
    match value {
        Value::Struct { fields, .. } => {
            for field in fields {
                if SINGLE_CLIP_FIELDS.iter().any(|name| h(name) == field.name) {
                    out.extend(field.value.as_u32());
                } else if field.name == h(CLIP_LIST_FIELD) {
                    out.extend(
                        field
                            .value
                            .items()
                            .unwrap_or_default()
                            .iter()
                            .filter_map(Value::as_u32),
                    );
                } else {
                    clip_refs(&field.value, out);
                }
            }
        }
        Value::List { items, .. } => items.iter().for_each(|item| clip_refs(item, out)),
        Value::Optional {
            value: Some(inner), ..
        } => clip_refs(inner, out),
        Value::Map { entries, .. } => entries.iter().for_each(|(_, v)| clip_refs(v, out)),
        Value::Raw { .. } | Value::Optional { value: None, .. } => {}
    }
}

fn rename_refs(value: &mut Value, renames: &BTreeMap<u32, u32>) {
    let rename = |v: &mut Value| {
        if let Some(new) = v.as_u32().and_then(|old| renames.get(&old)) {
            *v = hash_value(*new);
        }
    };
    match value {
        Value::Struct { fields, .. } => {
            for field in fields {
                if SINGLE_CLIP_FIELDS.iter().any(|name| h(name) == field.name) {
                    rename(&mut field.value);
                } else if field.name == h(CLIP_LIST_FIELD) {
                    if let Some(items) = field.value.items_mut() {
                        items.iter_mut().for_each(rename);
                    }
                } else {
                    rename_refs(&mut field.value, renames);
                }
            }
        }
        Value::List { items, .. } => items.iter_mut().for_each(|item| rename_refs(item, renames)),
        Value::Optional {
            value: Some(inner), ..
        } => rename_refs(inner, renames),
        Value::Map { entries, .. } => entries
            .iter_mut()
            .for_each(|(_, v)| rename_refs(v, renames)),
        Value::Raw { .. } | Value::Optional { value: None, .. } => {}
    }
}

fn add_event(clip: &mut Value, event: &Value) -> Result<(), ClassicError> {
    let fields = clip
        .fields_mut()
        .ok_or_else(|| ClassicError::Bin("an atomic clip is not a structure".into()))?;
    let key = hash_value(h("DekanGearSwap"));
    match tree::field_mut(fields, h("mEventDataMap")) {
        Some(Value::Map { entries, .. }) => {
            entries.retain(|(k, _)| k != &key);
            entries.push((key, event.clone()));
        }
        Some(_) => {
            return Err(ClassicError::Bin(
                "a clip's event table is not a map".into(),
            ));
        }
        None => tree::set_field(
            fields,
            h("mEventDataMap"),
            Value::Map {
                key: FIELD_HASH,
                value: FIELD_POINTER,
                entries: vec![(key, event.clone())],
            },
        ),
    }
    Ok(())
}

struct Copier<'a> {
    form: usize,
    event: &'a Value,
    copies: BTreeMap<u32, u32>,
}

impl Copier<'_> {
    fn copy(
        &mut self,
        entries: &mut ClipEntries,
        key: u32,
        depth: usize,
    ) -> Result<u32, ClassicError> {
        if let Some(copy) = self.copies.get(&key) {
            return Ok(*copy);
        }
        if depth > MAX_CLIP_DEPTH {
            return Err(ClassicError::Bin("animation clips nested too deep".into()));
        }
        let at = clip_at(entries, key)
            .ok_or_else(|| ClassicError::Bin(format!("clip {key:08x} is not in the graph")))?;
        let new_key = h(&format!("DekanGear{}_{key:08x}", self.form));
        self.copies.insert(key, new_key);
        let mut clip = entries[at].1.clone();
        if clip.class() == Some(h("AtomicClipData")) {
            add_event(&mut clip, self.event)?;
        } else {
            let mut refs = Vec::new();
            clip_refs(&clip, &mut refs);
            let mut renames = BTreeMap::new();
            for child in refs {
                if clip_at(entries, child).is_some() {
                    renames.insert(child, self.copy(entries, child, depth + 1)?);
                }
            }
            rename_refs(&mut clip, &renames);
        }
        entries.push((hash_value(new_key), clip));
        Ok(new_key)
    }
}

pub fn add_toggle(
    graph_bin: &[u8],
    graph_key: u32,
    swaps: &[GearSwap],
) -> Result<Option<Vec<u8>>, ClassicError> {
    let Some(markers) = markers(swaps) else {
        return Ok(None);
    };
    let mut file = parse_prop_file(graph_bin).map_err(bin_error)?;
    let Some(at) = file.entries.iter().position(|e| e.key_hash == graph_key) else {
        return Ok(None);
    };
    let mut graph = tree::parse_fields(&file.entries[at].body).map_err(bin_error)?;
    let Some(Value::Map {
        key: FIELD_HASH,
        entries,
        ..
    }) = tree::field_mut(&mut graph, h("mClipDataMap"))
    else {
        return Ok(None);
    };
    if clip_at(entries, h(TOGGLE_CLIP)).is_some() {
        return Ok(None);
    }

    let shown_anywhere: BTreeSet<u32> = swaps.iter().flat_map(|s| s.show.iter().copied()).collect();
    let mut carriers = Vec::with_capacity(swaps.len());
    for (form, swap) in swaps.iter().enumerate() {
        let hide: Vec<u32> = shown_anywhere
            .iter()
            .chain(swap.hide.iter())
            .copied()
            .filter(|part| !swap.show.contains(part))
            .collect::<BTreeSet<u32>>()
            .into_iter()
            .collect();
        let event = visibility_event(&swap.show, &hide);
        let start = swap
            .equip
            .as_deref()
            .map(h)
            .into_iter()
            .chain(CARRIER_FALLBACKS.iter().map(|name| h(name)))
            .find(|key| clip_at(entries, *key).is_some());
        let Some(start) = start else {
            return Ok(None);
        };
        let mut copier = Copier {
            form,
            event: &event,
            copies: BTreeMap::new(),
        };
        carriers.push(copier.copy(entries, start, 0)?);
    }

    let count = swaps.len();
    let link_name = |i: usize| {
        if i == 0 {
            h(TOGGLE_CLIP)
        } else {
            h(&format!("DekanToggle{i}"))
        }
    };
    for i in 0..count {
        let otherwise = if i + 1 < count {
            link_name(i + 1)
        } else {
            carriers[1]
        };
        entries.push((
            hash_value(link_name(i)),
            condition_on_part(markers[i], carriers[(i + 1) % count], otherwise),
        ));
    }

    file.entries[at].body = tree::write_fields(&graph).map_err(bin_error)?;
    serialize_prop_file(&file).map(Some).map_err(bin_error)
}

fn part_visible(kind: u8, part: u32) -> Value {
    Value::Struct {
        kind,
        class: h("SubmeshVisibilityBoolDriver"),
        fields: vec![
            named("Submeshes", hash_list(&[part])),
            named(
                "VISIBLE",
                Value::Raw {
                    kind: FIELD_BOOL,
                    bytes: vec![1],
                },
            ),
        ],
    }
}

fn gear_index(driver: &Value) -> u8 {
    driver
        .fields()
        .and_then(|f| tree::field(f, h("mGearIndex")))
        .and_then(|v| match v {
            Value::Raw { bytes, .. } => bytes.first().copied(),
            _ => None,
        })
        .unwrap_or(0)
}

fn children_mut(value: &mut Value) -> Vec<&mut Value> {
    match value {
        Value::Struct { fields, .. } => fields.iter_mut().map(|f| &mut f.value).collect(),
        Value::List { items, .. } => items.iter_mut().collect(),
        Value::Optional {
            value: Some(inner), ..
        } => vec![inner.as_mut()],
        Value::Map { entries, .. } => entries.iter_mut().map(|(_, v)| v).collect(),
        Value::Raw { .. } | Value::Optional { value: None, .. } => Vec::new(),
    }
}

fn highest_gear_index(value: &mut Value, depth: usize) -> Result<Option<u8>, ClassicError> {
    if depth > MAX_CLIP_DEPTH {
        return Err(ClassicError::Bin("drivers nested too deep".into()));
    }
    if value.class() == Some(h("HasGearDynamicMaterialBoolDriver")) {
        return Ok(Some(gear_index(value)));
    }
    let mut highest = None;
    for child in children_mut(value) {
        highest = highest.max(highest_gear_index(child, depth + 1)?);
    }
    Ok(highest)
}

fn redrive(value: &mut Value, markers: &[u32], depth: usize) -> usize {
    if value.class() == Some(h("HasGearDynamicMaterialBoolDriver")) {
        if let Some(part) = markers.get(usize::from(gear_index(value))) {
            *value = part_visible(value.kind(), *part);
            return 1;
        }
        return 0;
    }
    if depth > MAX_CLIP_DEPTH {
        return 0;
    }
    children_mut(value)
        .into_iter()
        .map(|child| redrive(child, markers, depth + 1))
        .sum()
}

pub fn drive_by_parts(
    bin: &[u8],
    markers: &[u32],
) -> Result<Option<(Vec<u8>, usize)>, ClassicError> {
    let needle = h("HasGearDynamicMaterialBoolDriver").to_le_bytes();
    let holds_driver = |bytes: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if !holds_driver(bin) {
        return Ok(None);
    }
    let mut file = parse_prop_file(bin).map_err(bin_error)?;
    let mut parsed = Vec::new();
    let mut highest = None;
    for (at, entry) in file.entries.iter().enumerate() {
        if !holds_driver(&entry.body) {
            continue;
        }
        let mut fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
        for field in &mut fields {
            highest = highest.max(highest_gear_index(&mut field.value, 0)?);
        }
        parsed.push((at, fields));
    }
    match highest {
        Some(index) if usize::from(index) < markers.len() => {}
        _ => return Ok(None),
    }
    let mut total = 0;
    for (at, mut fields) in parsed {
        let changed: usize = fields
            .iter_mut()
            .map(|field| redrive(&mut field.value, markers, 0))
            .sum();
        if changed > 0 {
            file.entries[at].body = tree::write_fields(&fields).map_err(bin_error)?;
            total += changed;
        }
    }
    if total == 0 {
        return Ok(None);
    }
    serialize_prop_file(&file)
        .map(|bytes| Some((bytes, total)))
        .map_err(bin_error)
}

#[cfg(test)]
#[path = "gear_toggle_tests.rs"]
mod tests;
