use std::collections::BTreeSet;

use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};

use crate::error::ClassicError;
use crate::gear_toggle::{bin_error, h, hash_list, named};
use crate::vfx_markers::{FIELD_EMBED, embed, raw_text, text, walk};

const SKIN_CLASS: &str = "SkinCharacterDataProperties";
const OVERRIDE_CLASS: &str = "SkinMeshDataProperties_MaterialOverride";
const PART_LOOK: [&str; 2] = ["texture", "material"];
const KEPT_LOOK: [&str; 2] = ["skinScale", "selfIllumination"];
const FIELD_LIST: u8 = 0x80;

#[derive(Debug, Clone, PartialEq)]
pub struct GearModel {
    pub mesh: Option<String>,
    pub skeleton: Option<String>,
    pub scale: Option<Value>,
}

fn gear_mesh(fields: &[Field]) -> Option<&[Field]> {
    tree::field(fields, h("mGearData"))
        .and_then(Value::fields)
        .and_then(|data| tree::field(data, h("skinMeshProperties")))
        .and_then(Value::fields)
}

pub fn gear_models(gear_bodies: &[Vec<u8>]) -> Result<Vec<GearModel>, ClassicError> {
    gear_bodies
        .iter()
        .map(|body| {
            let fields = tree::parse_fields(body).map_err(bin_error)?;
            let mesh = gear_mesh(&fields).unwrap_or_default();
            Ok(GearModel {
                mesh: tree::field(mesh, h("simpleSkin")).and_then(raw_text),
                skeleton: tree::field(mesh, h("skeleton")).and_then(raw_text),
                scale: tree::field(mesh, h("skinScale")).cloned(),
            })
        })
        .collect()
}

fn part_overrides(mesh: &[Field], parts: &[String]) -> Vec<Value> {
    let look: Vec<Field> = PART_LOOK
        .iter()
        .filter_map(|name| tree::field(mesh, h(name)).map(|value| named(name, value.clone())))
        .collect();
    let mut entries: Vec<Value> = if look.is_empty() {
        Vec::new()
    } else {
        parts
            .iter()
            .map(|part| {
                let mut fields = vec![named("submesh", text(part))];
                fields.extend(look.iter().cloned());
                embed(OVERRIDE_CLASS, fields)
            })
            .collect()
    };
    entries.extend(
        tree::field(mesh, h("materialOverride"))
            .and_then(Value::items)
            .unwrap_or_default()
            .iter()
            .cloned(),
    );
    entries
}

fn with_hashes(data: &mut Vec<Field>, name: &str, added: &[u32]) {
    let mut hashes: Vec<u32> = tree::field(data, h(name))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default();
    for hash in added {
        if !hashes.contains(hash) {
            hashes.push(*hash);
        }
    }
    tree::set_field(data, h(name), hash_list(&hashes));
}

pub fn as_part_gears(
    gear_bodies: &[Vec<u8>],
    parts: &[Vec<String>],
) -> Result<Vec<Vec<u8>>, ClassicError> {
    if gear_bodies.len() != parts.len() {
        return Err(ClassicError::Bin(format!(
            "{} gears but {} part lists",
            gear_bodies.len(),
            parts.len()
        )));
    }
    let hashes: Vec<Vec<u32>> = parts
        .iter()
        .map(|own| own.iter().map(|p| h(p)).collect())
        .collect();
    gear_bodies
        .iter()
        .enumerate()
        .map(|(form, body)| {
            let mut fields = tree::parse_fields(body).map_err(bin_error)?;
            let old_mesh = gear_mesh(&fields).unwrap_or_default();
            let mut mesh: Vec<Field> = KEPT_LOOK
                .iter()
                .filter_map(|name| tree::field(old_mesh, h(name)).map(|v| named(name, v.clone())))
                .collect();
            let overrides = part_overrides(old_mesh, &parts[form]);
            if !overrides.is_empty() {
                mesh.push(named(
                    "materialOverride",
                    Value::List {
                        kind: FIELD_LIST,
                        element: FIELD_EMBED,
                        items: overrides,
                    },
                ));
            }
            let others: Vec<u32> = hashes
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != form)
                .flat_map(|(_, own)| own.iter().copied())
                .collect();
            let data = tree::field_mut(&mut fields, h("mGearData"))
                .and_then(Value::fields_mut)
                .ok_or_else(|| ClassicError::Bin(format!("gear {form} has no gear data")))?;
            tree::set_field(
                data,
                h("skinMeshProperties"),
                embed("SkinMeshDataProperties", mesh),
            );
            with_hashes(data, "mCharacterSubmeshesToShow", &hashes[form]);
            with_hashes(data, "mCharacterSubmeshesToHide", &others);
            tree::write_fields(&fields).map_err(bin_error)
        })
        .collect()
}

pub fn hidden_by_clips(bin: &[u8]) -> Result<BTreeSet<u32>, ClassicError> {
    let file = parse_prop_file(bin).map_err(bin_error)?;
    let (event_class, hide_list) = (h("SubmeshVisibilityEventData"), h("mHideSubmeshList"));
    let mut hidden = BTreeSet::new();
    for entry in &file.entries {
        for field in tree::parse_fields(&entry.body).map_err(bin_error)? {
            walk(&field.value, &mut |value| {
                if let Value::Struct { class, fields, .. } = value
                    && *class == event_class
                    && let Some(parts) = tree::field(fields, hide_list).and_then(Value::items)
                {
                    hidden.extend(parts.iter().filter_map(Value::as_u32));
                }
            });
        }
    }
    Ok(hidden)
}

pub fn marker_parts(
    parts: &[Vec<String>],
    sizes: impl Fn(&str) -> u32,
    hidden: &BTreeSet<u32>,
) -> Result<Vec<u32>, ClassicError> {
    parts
        .iter()
        .enumerate()
        .skip(1)
        .map(|(form, own)| {
            own.iter()
                .filter(|part| !hidden.contains(&h(part)))
                .max_by_key(|part| sizes(part))
                .map(|part| h(part))
                .ok_or_else(|| {
                    ClassicError::Mesh(format!(
                        "every part of form {form} is hidden by some clip, so none can mark it"
                    ))
                })
        })
        .collect()
}

pub fn point_at(skin_bin: &[u8], mesh: &str, skeleton: &str) -> Result<Vec<u8>, ClassicError> {
    let mut file = parse_prop_file(skin_bin).map_err(bin_error)?;
    let entry = file
        .entries
        .iter_mut()
        .find(|e| e.class_hash == h(SKIN_CLASS))
        .ok_or_else(|| ClassicError::Bin("the skin bin has no skin object".into()))?;
    let mut fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
    let properties = tree::field_mut(&mut fields, h("skinMeshProperties"))
        .and_then(Value::fields_mut)
        .ok_or_else(|| ClassicError::Bin("the skin names no mesh properties".into()))?;
    tree::set_field(properties, h("simpleSkin"), text(mesh));
    tree::set_field(properties, h("skeleton"), text(skeleton));
    entry.body = tree::write_fields(&fields).map_err(bin_error)?;
    serialize_prop_file(&file).map_err(bin_error)
}

#[must_use]
pub fn merged_path(original: &str) -> String {
    let name_at = original.rfind('/').map_or(0, |slash| slash + 1);
    match original[name_at..].rfind('.') {
        Some(dot) => {
            let (stem, extension) = original.split_at(name_at + dot);
            format!("{stem}_DekanForms{extension}")
        }
        None => format!("{original}_DekanForms"),
    }
}

#[cfg(test)]
mod tests;
