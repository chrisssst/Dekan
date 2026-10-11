use std::collections::BTreeMap;

use dekan_wad::hash::{prop_key_hash, wad_path_hash};
use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{PropFile, parse_prop_file, serialize_prop_file};
use dekan_wad::wad::WadFile;

use crate::error::ClassicError;

const SKIN_DATA_CLASS: u32 = 0x9b67_e9f6;
const FIELD_LIST: u8 = 0x80;
const FIELD_POINTER: u8 = 0x82;
const FIELD_STRING: u8 = 16;
const MAX_DRIVER_DEPTH: usize = 128;

fn h(name: &str) -> u32 {
    prop_key_hash(name)
}

fn bin_error(e: impl std::fmt::Display) -> ClassicError {
    ClassicError::Bin(e.to_string())
}

fn skin_fields(entry_body: &[u8]) -> Result<Vec<Field>, ClassicError> {
    tree::parse_fields(entry_body).map_err(bin_error)
}

pub fn gear_keys(skin_bin: &[u8]) -> Result<Vec<u32>, ClassicError> {
    let parsed = parse_prop_file(skin_bin).map_err(bin_error)?;
    let Some(skin) = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
    else {
        return Ok(Vec::new());
    };
    let fields = skin_fields(&skin.body)?;
    let gears = tree::field(&fields, h("skinUpgradeData"))
        .and_then(Value::fields)
        .and_then(|upgrade| tree::field(upgrade, h("mGearSkinUpgrades")))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default();
    Ok(gears)
}

pub fn find_linked_object(
    wad: &WadFile,
    links: &[String],
    key: u32,
) -> Result<Option<Vec<u8>>, ClassicError> {
    for link in links {
        let Some(bytes) = wad.read(wad_path_hash(&link.to_ascii_lowercase()))? else {
            continue;
        };
        let Ok(bin) = parse_prop_file(&bytes) else {
            continue;
        };
        if let Some(entry) = bin.entries.into_iter().find(|e| e.key_hash == key) {
            return Ok(Some(entry.body));
        }
    }
    Ok(None)
}

pub fn skn_submesh_names(skn: &[u8]) -> Result<Vec<String>, ClassicError> {
    const MAGIC: u32 = 0x0011_2233;
    const RECORD: usize = 80;
    const NAME: usize = 64;
    let u32_at = |at: usize| {
        skn.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| ClassicError::Bin(format!("mesh truncated at offset {at}")))
    };
    if u32_at(0)? != MAGIC {
        return Err(ClassicError::Bin("not a skinned mesh".into()));
    }
    let major = skn
        .get(4..6)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| ClassicError::Bin("mesh truncated in its version".into()))?;
    if major == 0 {
        return Ok(Vec::new());
    }
    let count = u32_at(8)? as usize;
    let table = count
        .checked_mul(RECORD)
        .and_then(|len| len.checked_add(12))
        .filter(|end| *end <= skn.len())
        .ok_or_else(|| {
            ClassicError::Bin(format!("mesh declares {count} submeshes past its end"))
        })?;
    Ok(skn[12..table]
        .chunks_exact(RECORD)
        .map(|record| {
            let name = &record[..NAME];
            let end = name.iter().position(|b| *b == 0).unwrap_or(NAME);
            String::from_utf8_lossy(&name[..end]).into_owned()
        })
        .collect())
}

fn string_value(text: &str) -> Result<Value, ClassicError> {
    let len = u16::try_from(text.len()).map_err(|_| ClassicError::Bin("string too long".into()))?;
    let mut bytes = len.to_le_bytes().to_vec();
    bytes.extend_from_slice(text.as_bytes());
    Ok(Value::Raw {
        kind: FIELD_STRING,
        bytes,
    })
}

fn string_of(value: &Value) -> Option<String> {
    match value {
        Value::Raw { kind, bytes } if *kind == FIELD_STRING && bytes.len() >= 2 => {
            Some(String::from_utf8_lossy(&bytes[2..]).into_owned())
        }
        _ => None,
    }
}

fn hashes_in(fields: &[Field], name: &str) -> Vec<u32> {
    tree::field(fields, h(name))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default()
}

fn apply_visibility(
    mesh: &mut Vec<Field>,
    gear: &[Field],
    submeshes: &[String],
) -> Result<(), ClassicError> {
    let show = hashes_in(gear, "mCharacterSubmeshesToShow");
    let hide = hashes_in(gear, "mCharacterSubmeshesToHide");
    if show.is_empty() && hide.is_empty() {
        return Ok(());
    }
    let current = tree::field(mesh, h("initialSubmeshToHide"))
        .and_then(string_of)
        .unwrap_or_default();
    let mut hidden: Vec<String> = current
        .split_whitespace()
        .filter(|name| !show.contains(&h(name)))
        .map(str::to_owned)
        .collect();
    for name in hide
        .iter()
        .filter_map(|hash| submeshes.iter().find(|name| h(name) == *hash))
    {
        if !hidden.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            hidden.push(name.clone());
        }
    }
    tree::set_field(
        mesh,
        h("initialSubmeshToHide"),
        string_value(&hidden.join(" "))?,
    );
    Ok(())
}

fn submesh_of(value: &Value) -> Option<String> {
    value
        .fields()
        .and_then(|f| tree::field(f, h("submesh")))
        .and_then(string_of)
}

fn merge_mesh(skin: &mut Vec<Field>, gear_mesh: &Value) -> Result<(), ClassicError> {
    let Some(gear_fields) = gear_mesh.fields() else {
        return Ok(());
    };
    if tree::field(skin, h("skinMeshProperties")).is_none() {
        tree::set_field(
            skin,
            h("skinMeshProperties"),
            Value::Struct {
                kind: gear_mesh.kind(),
                class: gear_mesh.class().unwrap_or(0),
                fields: Vec::new(),
            },
        );
    }
    let mesh = tree::field_mut(skin, h("skinMeshProperties"))
        .and_then(Value::fields_mut)
        .ok_or_else(|| ClassicError::Bin("skinMeshProperties is not a structure".into()))?;
    for field in gear_fields {
        if field.name != h("materialOverride") {
            tree::set_field(mesh, field.name, field.value.clone());
            continue;
        }
        let Some(overrides) = field.value.items() else {
            continue;
        };
        match tree::field_mut(mesh, field.name).and_then(Value::items_mut) {
            Some(existing) => {
                let replaced: Vec<String> = overrides.iter().filter_map(submesh_of).collect();
                existing
                    .retain(|item| submesh_of(item).is_none_or(|name| !replaced.contains(&name)));
                existing.extend(overrides.iter().cloned());
            }
            None => tree::set_field(mesh, field.name, field.value.clone()),
        }
    }
    Ok(())
}

fn constant_driver(kind: u8, truth: bool) -> Value {
    let class = if truth {
        h("AllTrueMaterialDriver")
    } else {
        h("OneTrueMaterialDriver")
    };
    Value::Struct {
        kind,
        class,
        fields: vec![Field {
            name: h("mDrivers"),
            value: Value::List {
                kind: FIELD_LIST,
                element: FIELD_POINTER,
                items: Vec::new(),
            },
        }],
    }
}

fn fix_gear_drivers(
    value: &mut Value,
    gear_index: u32,
    depth: usize,
) -> Result<usize, ClassicError> {
    if depth > MAX_DRIVER_DEPTH {
        return Err(ClassicError::Bin("drivers nested too deep".into()));
    }
    let mut fixed = 0;
    if value.class() == Some(h("HasGearDynamicMaterialBoolDriver")) {
        let index = value
            .fields()
            .and_then(|f| tree::field(f, h("mGearIndex")))
            .and_then(|v| match v {
                Value::Raw { bytes, .. } => bytes.first().copied(),
                _ => None,
            })
            .unwrap_or(0);
        *value = constant_driver(value.kind(), u32::from(index) == gear_index);
        return Ok(1);
    }
    match value {
        Value::Struct { fields, .. } => {
            for field in fields {
                fixed += fix_gear_drivers(&mut field.value, gear_index, depth + 1)?;
            }
        }
        Value::List { items, .. } => {
            for item in items {
                fixed += fix_gear_drivers(item, gear_index, depth + 1)?;
            }
        }
        Value::Optional {
            value: Some(inner), ..
        } => {
            fixed += fix_gear_drivers(inner, gear_index, depth + 1)?;
        }
        Value::Map { entries, .. } => {
            for (_, v) in entries {
                fixed += fix_gear_drivers(v, gear_index, depth + 1)?;
            }
        }
        Value::Raw { .. } | Value::Optional { value: None, .. } => {}
    }
    Ok(fixed)
}

pub struct GearForm<'a> {
    pub index: u32,
    pub gear_body: &'a [u8],
    pub submeshes: &'a [String],
}

pub fn bake_form(file: &mut PropFile, form: &GearForm<'_>) -> Result<(), ClassicError> {
    let gear = tree::parse_fields(form.gear_body).map_err(bin_error)?;
    let gear_data = tree::field(&gear, h("mGearData"))
        .and_then(Value::fields)
        .map(<[Field]>::to_vec)
        .unwrap_or_default();

    let skin_at = file
        .entries
        .iter()
        .position(|e| e.class_hash == SKIN_DATA_CLASS)
        .ok_or_else(|| ClassicError::Bin("no skin object to bake a form into".into()))?;
    let mut skin = skin_fields(&file.entries[skin_at].body)?;

    if let Some(upgrade) =
        tree::field_mut(&mut skin, h("skinUpgradeData")).and_then(Value::fields_mut)
    {
        tree::remove_field(upgrade, h("mGearSkinUpgrades"));
    }
    if let Some(mesh) = tree::field(&gear_data, h("skinMeshProperties")) {
        merge_mesh(&mut skin, mesh)?;
    }
    if let Some(mesh) =
        tree::field_mut(&mut skin, h("skinMeshProperties")).and_then(Value::fields_mut)
    {
        apply_visibility(mesh, &gear_data, form.submeshes)?;
    }
    let enabled = tree::field(&gear_data, h("EnableOverrideIdleEffects"))
        .is_some_and(|v| matches!(v, Value::Raw { bytes, .. } if bytes.first() == Some(&1)));
    if enabled {
        match tree::field(&gear_data, h("OverrideIdleEffects")) {
            Some(effects) => tree::set_field(&mut skin, h("idleParticlesEffects"), effects.clone()),
            None => {
                if let Some(Value::List { items, .. }) =
                    tree::field_mut(&mut skin, h("idleParticlesEffects"))
                {
                    items.clear();
                }
            }
        }
    }
    if let Some(icon) = tree::field(&gear_data, h("mSelfOnlyPortraitIcon")) {
        tree::set_field(&mut skin, h("iconAvatar"), icon.clone());
    }
    for field in &mut skin {
        fix_gear_drivers(&mut field.value, form.index, 0)?;
    }

    let resolver_key = tree::field(&skin, h("mResourceResolver")).and_then(Value::as_u32);
    file.entries[skin_at].body = tree::write_fields(&skin).map_err(bin_error)?;

    let gear_map = tree::field(&gear_data, h("mVFXResourceResolver"))
        .and_then(Value::fields)
        .and_then(|r| tree::field(r, h("resourceMap")));
    if let (Some(Value::Map { entries: extra, .. }), Some(key)) = (gear_map, resolver_key) {
        if let Some(resolver) = file.entries.iter_mut().find(|e| e.key_hash == key) {
            let mut fields = tree::parse_fields(&resolver.body).map_err(bin_error)?;
            if let Some(Value::Map { entries, .. }) = tree::field_mut(&mut fields, h("resourceMap"))
            {
                let mut by_key: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
                for (i, (k, _)) in entries.iter().enumerate() {
                    if let Value::Raw { bytes, .. } = k {
                        by_key.insert(bytes.clone(), i);
                    }
                }
                for (k, v) in extra {
                    let existing = match k {
                        Value::Raw { bytes, .. } => by_key.get(bytes).copied(),
                        _ => None,
                    };
                    match existing {
                        Some(i) => entries[i].1 = v.clone(),
                        None => entries.push((k.clone(), v.clone())),
                    }
                }
            }
            resolver.body = tree::write_fields(&fields).map_err(bin_error)?;
        }
    }
    Ok(())
}

pub fn strip_gear_indicators(skin_bin: &[u8]) -> Result<Vec<u8>, ClassicError> {
    let mut file = parse_prop_file(skin_bin).map_err(bin_error)?;
    let Some(at) = file
        .entries
        .iter()
        .position(|e| e.class_hash == SKIN_DATA_CLASS)
    else {
        return Ok(skin_bin.to_vec());
    };
    let mut skin = skin_fields(&file.entries[at].body)?;
    let stripped =
        match tree::field_mut(&mut skin, h("skinUpgradeData")).and_then(Value::fields_mut) {
            Some(upgrade) => tree::remove_field(upgrade, h("mGearSkinUpgrades")).is_some(),
            None => false,
        };
    if !stripped {
        return Ok(skin_bin.to_vec());
    }
    file.entries[at].body = tree::write_fields(&skin).map_err(bin_error)?;
    serialize_prop_file(&file).map_err(bin_error)
}
