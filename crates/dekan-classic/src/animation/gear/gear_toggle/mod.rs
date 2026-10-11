use std::collections::BTreeMap;

use dekan_wad::hash::prop_key_hash;
use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};

use crate::error::ClassicError;

const FIELD_BOOL: u8 = 1;
pub(crate) const FIELD_HASH: u8 = 17;
const FIELD_LIST: u8 = 0x80;
const FIELD_POINTER: u8 = 0x82;
const MAX_CLIP_DEPTH: usize = 64;
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

pub(crate) fn hash_list(hashes: &[u32]) -> Value {
    Value::List {
        kind: FIELD_LIST,
        element: FIELD_HASH,
        items: hashes.iter().copied().map(hash_value).collect(),
    }
}

pub(crate) fn pointer(class: &str, fields: Vec<Field>) -> Value {
    Value::Struct {
        kind: FIELD_POINTER,
        class: h(class),
        fields,
    }
}

pub(crate) fn named(name: &str, value: Value) -> Field {
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
    pub transition: Option<u32>,
}

pub fn gear_swap(gear_body: &[u8]) -> Result<GearSwap, ClassicError> {
    let gear = tree::parse_fields(gear_body).map_err(bin_error)?;
    let data = tree::field(&gear, h("mGearData"))
        .and_then(Value::fields)
        .unwrap_or_default();
    let equip = tree::field(data, h("mEquipAnimation"))
        .and_then(crate::vfx_markers::raw_text)
        .filter(|name| !name.is_empty());
    Ok(GearSwap {
        show: hashes_in(data, "mCharacterSubmeshesToShow"),
        hide: hashes_in(data, "mCharacterSubmeshesToHide"),
        equip,
        transition: None,
    })
}

pub(crate) fn visibility_event(show: &[u32], hide: &[u32]) -> Value {
    pointer(
        "SubmeshVisibilityEventData",
        vec![
            named("mShowSubmeshList", hash_list(show)),
            named("mHideSubmeshList", hash_list(hide)),
        ],
    )
}

pub(crate) fn condition_on_part(part: u32, visible: u32, otherwise: u32) -> Value {
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

pub(crate) fn rename_refs(value: &mut Value, renames: &BTreeMap<u32, u32>) {
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

pub(crate) fn add_event(clip: &mut Value, event: &Value) -> Result<(), ClassicError> {
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

pub(crate) fn part_visible(kind: u8, part: u32) -> Value {
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

fn redrive(value: &mut Value, drivers: &[Value], depth: usize) -> usize {
    if value.class() == Some(h("HasGearDynamicMaterialBoolDriver")) {
        let Some(Value::Struct { class, fields, .. }) = drivers.get(usize::from(gear_index(value)))
        else {
            return 0;
        };
        *value = Value::Struct {
            kind: value.kind(),
            class: *class,
            fields: fields.clone(),
        };
        return 1;
    }
    if depth > MAX_CLIP_DEPTH {
        return 0;
    }
    children_mut(value)
        .into_iter()
        .map(|child| redrive(child, drivers, depth + 1))
        .sum()
}

pub fn drive_by_parts(
    bin: &[u8],
    drivers: &[Value],
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
        Some(index) if usize::from(index) < drivers.len() => {}
        _ => return Ok(None),
    }
    let mut total = 0;
    for (at, mut fields) in parsed {
        let changed: usize = fields
            .iter_mut()
            .map(|field| redrive(&mut field.value, drivers, 0))
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
mod tests;
