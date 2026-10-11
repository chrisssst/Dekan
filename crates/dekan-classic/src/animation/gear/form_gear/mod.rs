use std::collections::{BTreeMap, BTreeSet};

use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};

use crate::error::ClassicError;
use crate::form_state::{form_active, form_parts};
use crate::gear_toggle::{GearSwap, bin_error, h, named, pointer};
use crate::vfx_markers::{FIELD_EMBED, FIELD_LIST2, FIELD_POINTER, embed, text};

const SKIN_CLASS: &str = "SkinCharacterDataProperties";
const MESH_SWAP: [&str; 2] = ["simpleSkin", "skeleton"];
const WHOLE_LOOK: [&str; 4] = ["texture", "material", "skinScale", "selfIllumination"];
const IDLE_FIELDS: [&str; 3] = ["effectKey", "boneName", "targetBoneName"];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GearCoverage {
    pub idle_forms: usize,
    pub material_parts: usize,
    pub part_copies: usize,
    pub per_form_look: BTreeSet<&'static str>,
    pub mesh_swap: bool,
    pub script_states: usize,
    pub model: Option<ModelOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelOutcome {
    Merged { forms: usize, twins: usize },
    Refused(String),
}

fn gear_data(body: &[u8]) -> Result<Vec<Field>, ClassicError> {
    let fields = tree::parse_fields(body).map_err(bin_error)?;
    Ok(tree::field(&fields, h("mGearData"))
        .and_then(Value::fields)
        .map(<[Field]>::to_vec)
        .unwrap_or_default())
}

fn mesh_of(gear: &[Field]) -> Vec<Field> {
    tree::field(gear, h("skinMeshProperties"))
        .and_then(Value::fields)
        .map(<[Field]>::to_vec)
        .unwrap_or_default()
}

fn idle_override(gear: &[Field]) -> Option<Vec<Value>> {
    let enabled = matches!(
        tree::field(gear, h("EnableOverrideIdleEffects")),
        Some(Value::Raw { bytes, .. }) if bytes.first().is_some_and(|b| *b != 0)
    );
    enabled.then(|| {
        tree::field(gear, h("OverrideIdleEffects"))
            .and_then(Value::items)
            .map(<[Value]>::to_vec)
            .unwrap_or_default()
    })
}

fn differs(gears: &[Vec<Field>], name: &str) -> bool {
    let values: Vec<Option<Value>> = gears
        .iter()
        .map(|g| tree::field(&mesh_of(g), h(name)).cloned())
        .collect();
    values.iter().any(|v| *v != values[0])
}

fn submesh_of(entry: &Value) -> Option<String> {
    match entry.fields().and_then(|f| tree::field(f, h("submesh"))) {
        Some(Value::Raw { bytes, .. }) if bytes.len() >= 2 => {
            Some(String::from_utf8_lossy(&bytes[2..]).into_owned())
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartCopy {
    pub source: String,
    pub name: String,
    pub form: usize,
    pub material: Option<Value>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct MaterialPlan {
    pub overrides: Vec<Value>,
    pub copies: Vec<PartCopy>,
}

fn renamed(entry: &Value, name: &str) -> Value {
    let mut entry = entry.clone();
    if let Some(fields) = entry.fields_mut() {
        tree::set_field(fields, h("submesh"), text(name));
    }
    entry
}

pub fn material_plan(
    gear_bodies: &[Vec<u8>],
    swaps: &[GearSwap],
    initially_hidden: &BTreeSet<u32>,
) -> Result<MaterialPlan, ClassicError> {
    let gears = gear_bodies
        .iter()
        .map(|b| gear_data(b))
        .collect::<Result<Vec<_>, _>>()?;
    let mut by_part: BTreeMap<String, (String, Vec<Option<Value>>)> = BTreeMap::new();
    for (form, gear) in gears.iter().enumerate() {
        for entry in tree::field(&mesh_of(gear), h("materialOverride"))
            .and_then(Value::items)
            .unwrap_or_default()
        {
            let Some(name) = submesh_of(entry) else {
                continue;
            };
            let slot = by_part
                .entry(name.to_ascii_lowercase())
                .or_insert_with(|| (name.clone(), vec![None; gears.len()]));
            slot.1[form] = Some(entry.clone());
        }
    }
    let hidden: Vec<BTreeSet<u32>> = form_parts(swaps)
        .into_iter()
        .map(|(_, hide)| hide.into_iter().collect())
        .collect();
    let mut plan = MaterialPlan::default();
    for (name, per_form) in by_part.into_values() {
        let part = h(&name);
        let visible: Vec<usize> = (0..gears.len())
            .filter(|f| {
                !hidden[*f].contains(&part)
                    && (swaps[*f].show.contains(&part) || !initially_hidden.contains(&part))
            })
            .collect();
        let Some(first) = visible.first().copied() else {
            continue;
        };
        let base = per_form[first].clone();
        plan.overrides.extend(base.clone());
        for form in visible.into_iter().skip(1).filter(|f| per_form[*f] != base) {
            let copy = format!("{name}_F{form}");
            plan.copies.push(PartCopy {
                material: per_form[form].as_ref().map(|e| renamed(e, &copy)),
                source: name.clone(),
                name: copy,
                form,
            });
        }
    }
    Ok(plan)
}

pub fn with_copies(swaps: &[GearSwap], copies: &[PartCopy]) -> Vec<GearSwap> {
    let mut swaps = swaps.to_vec();
    for copy in copies {
        let (source, name) = (h(&copy.source), h(&copy.name));
        let swap = &mut swaps[copy.form];
        swap.show.retain(|p| *p != source);
        swap.show.push(name);
        swap.hide.push(source);
    }
    swaps
}

fn holds_class(value: &Value, class: u32) -> bool {
    match value {
        Value::Struct {
            class: c, fields, ..
        } => *c == class || fields.iter().any(|f| holds_class(&f.value, class)),
        Value::List { items, .. } => items.iter().any(|i| holds_class(i, class)),
        Value::Optional {
            value: Some(inner), ..
        } => holds_class(inner, class),
        Value::Map { entries, .. } => entries.iter().any(|(_, v)| holds_class(v, class)),
        Value::Raw { .. } | Value::Optional { value: None, .. } => false,
    }
}

fn script_states(skin: &[Field]) -> usize {
    tree::field(skin, h("PersistentEffectConditions"))
        .and_then(Value::items)
        .unwrap_or_default()
        .iter()
        .filter(|c| holds_class(c, h("HasBuffDynamicMaterialBoolDriver")))
        .count()
}

pub fn gear_coverage(
    source: &[u8],
    gear_bodies: &[Vec<u8>],
    swaps: &[GearSwap],
) -> Result<GearCoverage, ClassicError> {
    let gears = gear_bodies
        .iter()
        .map(|b| gear_data(b))
        .collect::<Result<Vec<_>, _>>()?;
    let file = parse_prop_file(source).map_err(bin_error)?;
    let skin = match file.entries.iter().find(|e| e.class_hash == h(SKIN_CLASS)) {
        Some(entry) => tree::parse_fields(&entry.body).map_err(bin_error)?,
        None => Vec::new(),
    };
    let hidden: BTreeSet<u32> = crate::form_marker::mesh_text(source, "initialSubmeshToHide")?
        .unwrap_or_default()
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '|' | ':'))
        .filter(|name| !name.is_empty())
        .map(h)
        .collect();
    let plan = material_plan(gear_bodies, swaps, &hidden)?;
    Ok(GearCoverage {
        idle_forms: gears.iter().filter(|g| idle_override(g).is_some()).count(),
        material_parts: plan.overrides.len(),
        part_copies: plan.copies.len(),
        per_form_look: WHOLE_LOOK
            .into_iter()
            .filter(|n| differs(&gears, n))
            .collect(),
        mesh_swap: MESH_SWAP.iter().any(|n| differs(&gears, n)),
        script_states: script_states(&skin),
        ..GearCoverage::default()
    })
}

fn as_persistent(idle: &Value) -> Value {
    let fields = idle
        .fields()
        .unwrap_or_default()
        .iter()
        .filter(|f| IDLE_FIELDS.iter().any(|n| h(n) == f.name))
        .cloned()
        .collect();
    embed("PersistentVfxData", fields)
}

pub fn apply_gears(
    skin0: &[u8],
    gear_bodies: &[Vec<u8>],
    markers: &[u32],
    plan: &MaterialPlan,
) -> Result<Option<Vec<u8>>, ClassicError> {
    let gears = gear_bodies
        .iter()
        .map(|b| gear_data(b))
        .collect::<Result<Vec<_>, _>>()?;
    if gears.len() < 2 || markers.len() + 1 != gears.len() {
        return Ok(None);
    }
    let mut file = parse_prop_file(skin0).map_err(bin_error)?;
    let Some(at) = file
        .entries
        .iter()
        .position(|e| e.class_hash == h(SKIN_CLASS))
    else {
        return Ok(None);
    };
    let mut skin = tree::parse_fields(&file.entries[at].body).map_err(bin_error)?;
    let mut changed = false;

    let overrides: Vec<Option<Vec<Value>>> = gears.iter().map(|g| idle_override(g)).collect();
    if overrides.iter().any(Option::is_some) {
        let own = tree::field(&skin, h("idleParticlesEffects"))
            .and_then(Value::items)
            .map(<[Value]>::to_vec)
            .unwrap_or_default();
        let conditions: Vec<Value> = overrides
            .into_iter()
            .enumerate()
            .filter_map(|(form, idle)| {
                let items: Vec<Value> = idle
                    .unwrap_or_else(|| own.clone())
                    .iter()
                    .map(as_persistent)
                    .collect();
                (!items.is_empty()).then(|| {
                    pointer(
                        "PersistentEffectConditionData",
                        vec![
                            named("OwnerCondition", form_active(form, markers)),
                            named(
                                "PersistentVfxs",
                                Value::List {
                                    kind: FIELD_LIST2,
                                    element: FIELD_EMBED,
                                    items,
                                },
                            ),
                        ],
                    )
                })
            })
            .collect();
        skin.retain(|f| f.name != h("idleParticlesEffects"));
        match tree::field_mut(&mut skin, h("PersistentEffectConditions")).and_then(Value::items_mut)
        {
            Some(items) => items.extend(conditions),
            None => tree::set_field(
                &mut skin,
                h("PersistentEffectConditions"),
                Value::List {
                    kind: FIELD_LIST2,
                    element: FIELD_POINTER,
                    items: conditions,
                },
            ),
        }
        changed = true;
    }

    let parts: Vec<Value> = plan
        .overrides
        .iter()
        .cloned()
        .chain(plan.copies.iter().filter_map(|c| c.material.clone()))
        .collect();
    if !parts.is_empty()
        && let Some(mesh) =
            tree::field_mut(&mut skin, h("skinMeshProperties")).and_then(Value::fields_mut)
    {
        let names: BTreeSet<String> = parts
            .iter()
            .filter_map(submesh_of)
            .map(|n| n.to_ascii_lowercase())
            .collect();
        match tree::field_mut(mesh, h("materialOverride")).and_then(Value::items_mut) {
            Some(items) => {
                items.retain(|e| {
                    submesh_of(e).is_none_or(|p| !names.contains(&p.to_ascii_lowercase()))
                });
                items.extend(parts);
            }
            None => tree::set_field(
                mesh,
                h("materialOverride"),
                Value::List {
                    kind: 0x80,
                    element: FIELD_EMBED,
                    items: parts,
                },
            ),
        }
        changed = true;
    }

    if !changed {
        return Ok(None);
    }
    file.entries[at].body = tree::write_fields(&skin).map_err(bin_error)?;
    serialize_prop_file(&file).map(Some).map_err(bin_error)
}

#[cfg(test)]
mod tests;
