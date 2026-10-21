use std::collections::{BTreeMap, BTreeSet, HashMap};

use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{PropEntry, parse_prop_file, serialize_prop_file};

use tracing::{debug, warn};

use crate::error::ClassicError;
use crate::gear_toggle::{bin_error, h, hash_value, named};

use crate::vfx_markers::{
    EMITTER_LISTS, FIELD_LINK, FIELD_LIST, FIELD_POINTER, FIELD_U8, MAX_DEPTH, STENCIL_EQUAL,
    embed, form_stencil, hold_markers, marker_systems, raw, text,
};

fn resource_map(value: &Value) -> BTreeMap<u32, u32> {
    match value
        .fields()
        .and_then(|f| tree::field(f, h("resourceMap")))
    {
        Some(Value::Map { entries, .. }) => entries
            .iter()
            .filter_map(|(k, v)| Some((k.as_u32()?, v.as_u32()?)))
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn gear_resolver(gear_body: &[u8]) -> Result<BTreeMap<u32, u32>, ClassicError> {
    let fields = tree::parse_fields(gear_body).map_err(bin_error)?;
    Ok(tree::field(&fields, h("mGearData"))
        .and_then(Value::fields)
        .and_then(|g| tree::field(g, h("mVFXResourceResolver")))
        .map(resource_map)
        .unwrap_or_default())
}

fn signature(emitter: &Value) -> Option<Vec<u8>> {
    tree::write_fields(&[Field {
        name: 0,
        value: emitter.clone(),
    }])
    .ok()
}

fn gated_key(path: u32, form: usize) -> u32 {
    h(&format!("Dekan/FormEffect{form}_{path:08x}"))
}

struct Gate<'a> {
    systems: &'a HashMap<u32, Vec<Field>>,
    targets: &'a BTreeMap<u32, Vec<Option<u32>>>,
    added: BTreeMap<u32, Vec<Field>>,
    stamped: usize,
    unresolved: usize,
}

impl Gate<'_> {
    fn stamp(&mut self, emitter: &mut Value, form: usize, depth: usize) {
        let Some(fields) = emitter.fields_mut() else {
            return;
        };
        fields.retain(|f| f.name != h("StencilReferenceId"));
        tree::set_field(fields, h("stencilMode"), raw(FIELD_U8, vec![STENCIL_EQUAL]));
        tree::set_field(
            fields,
            h("stencilRef"),
            raw(FIELD_U8, vec![form_stencil(form)]),
        );
        self.stamped += 1;
        let Some(ids) = tree::field_mut(fields, h("childParticleSetDefinition"))
            .and_then(Value::fields_mut)
            .and_then(|c| tree::field_mut(c, h("childrenIdentifiers")))
            .and_then(Value::items_mut)
        else {
            return;
        };
        for id in ids.iter_mut() {
            let Some(id) = id.fields_mut() else {
                continue;
            };
            let linked = tree::field(id, h("effect"))
                .and_then(Value::as_u32)
                .filter(|l| *l != 0);
            let by_key = tree::field(id, h("effectKey"))
                .and_then(Value::as_u32)
                .and_then(|k| self.targets.get(&k))
                .and_then(|t| t[form]);
            let Some(source) = linked.or(by_key) else {
                continue;
            };
            let Some(copy) = self.child(source, form, depth + 1) else {
                continue;
            };
            id.retain(|f| f.name != h("effectKey"));
            tree::set_field(
                id,
                h("effect"),
                raw(FIELD_LINK, copy.to_le_bytes().to_vec()),
            );
        }
    }

    fn child(&mut self, path: u32, form: usize, depth: usize) -> Option<u32> {
        let key = gated_key(path, form);
        if self.added.contains_key(&key) {
            return Some(key);
        }
        if depth > MAX_DEPTH {
            self.unresolved += 1;
            return None;
        }
        let Some(system) = self.systems.get(&path) else {
            self.unresolved += 1;
            return None;
        };
        let mut fields = system.clone();
        self.added.insert(key, Vec::new());
        for list in EMITTER_LISTS {
            if let Some(items) = tree::field_mut(&mut fields, h(list)).and_then(Value::items_mut) {
                let mut stamped: Vec<Value> = items.clone();
                for emitter in &mut stamped {
                    self.stamp(emitter, form, depth);
                }
                *items = stamped;
            }
        }
        rename(&mut fields, key);
        self.added.insert(key, fields);
        Some(key)
    }

    fn merge(&mut self, key: u32, forms: &[Option<u32>]) -> Option<(u32, Vec<Field>)> {
        let systems: Vec<Option<&Vec<Field>>> = forms
            .iter()
            .map(|t| t.and_then(|p| self.systems.get(&p)))
            .collect();
        let template = systems.iter().flatten().next()?;
        let mut merged: Vec<Field> = template
            .iter()
            .filter(|f| !EMITTER_LISTS.iter().any(|l| h(l) == f.name))
            .cloned()
            .collect();
        for list in EMITTER_LISTS {
            let copies: Vec<Vec<Value>> = systems
                .iter()
                .map(|s| {
                    s.and_then(|f| tree::field(f, h(list)))
                        .and_then(Value::items)
                        .map(<[Value]>::to_vec)
                        .unwrap_or_default()
                })
                .collect();
            let mut shared: HashMap<Vec<u8>, usize> = HashMap::new();
            if systems.iter().all(Option::is_some) {
                for emitter in &copies[0] {
                    let Some(sig) = signature(emitter) else {
                        continue;
                    };
                    let n = copies
                        .iter()
                        .map(|c| {
                            c.iter()
                                .filter(|e| signature(e).as_ref() == Some(&sig))
                                .count()
                        })
                        .min()
                        .unwrap_or(0);
                    shared.insert(sig, n);
                }
            }
            let mut items = Vec::new();
            for (form, emitters) in copies.into_iter().enumerate() {
                let mut used: HashMap<Vec<u8>, usize> = HashMap::new();
                for mut emitter in emitters {
                    if let Some(sig) = signature(&emitter)
                        && let Some(n) = shared.get(&sig).copied()
                    {
                        let u = used.entry(sig).or_default();
                        if *u < n {
                            *u += 1;
                            if form == 0 {
                                items.push(emitter);
                            }
                            continue;
                        }
                    }
                    self.stamp(&mut emitter, form, 0);
                    items.push(emitter);
                }
            }
            if !items.is_empty() {
                merged.push(named(
                    list,
                    Value::List {
                        kind: FIELD_LIST,
                        element: FIELD_POINTER,
                        items,
                    },
                ));
            }
        }
        let path = h(&format!("Dekan/FormEffects_{key:08x}"));
        rename(&mut merged, path);
        Some((path, merged))
    }
}

fn rename(fields: &mut Vec<Field>, path: u32) {
    let name = format!("Dekan/{path:08x}");
    for field in ["particleName", "particlePath"] {
        if tree::field(fields, h(field)).is_some() {
            tree::set_field(fields, h(field), text(&name));
        }
    }
    if tree::field(fields, h("objectPath")).is_some() {
        tree::set_field(fields, h("objectPath"), hash_value(path));
    }
}

pub struct FormEffects<'a> {
    pub bins: &'a [Vec<u8>],
    pub gear_bodies: &'a [Vec<u8>],
    pub markers: &'a [u32],
    pub bone: &'a str,
}

pub fn gate_form_effects(
    skin0: &[u8],
    plan: &FormEffects<'_>,
) -> Result<Option<(Vec<u8>, usize)>, ClassicError> {
    let forms = plan.gear_bodies.len();
    if forms < 2 || plan.markers.len() + 1 != forms {
        debug!(
            forms,
            markers = plan.markers.len(),
            "Form effects skipped: the markers do not match the forms"
        );
        return Ok(None);
    }
    let mut systems: HashMap<u32, Vec<Field>> = HashMap::new();
    let mut unparsed = 0usize;
    let mut first_error = None;
    for bin in plan.bins {
        let file = parse_prop_file(bin).map_err(bin_error)?;
        for entry in file
            .entries
            .iter()
            .filter(|e| e.class_hash == h("VfxSystemDefinitionData"))
        {
            match tree::parse_fields(&entry.body) {
                Ok(fields) => {
                    systems.entry(entry.key_hash).or_insert(fields);
                }
                Err(e) => {
                    unparsed += 1;
                    first_error.get_or_insert(e.to_string());
                }
            }
        }
    }
    if let Some(error) = first_error {
        warn!(unparsed, error = %error, "Effect systems unreadable; their form gating is skipped");
    }
    let gears = plan
        .gear_bodies
        .iter()
        .map(|b| gear_resolver(b))
        .collect::<Result<Vec<_>, _>>()?;

    let mut file = parse_prop_file(skin0).map_err(bin_error)?;
    let Some(skin_at) = file
        .entries
        .iter()
        .position(|e| e.class_hash == h("SkinCharacterDataProperties"))
    else {
        warn!("Form effects skipped: the skin bin has no skin object");
        return Ok(None);
    };
    let mut skin = tree::parse_fields(&file.entries[skin_at].body).map_err(bin_error)?;
    let Some(resolver_key) = tree::field(&skin, h("mResourceResolver")).and_then(Value::as_u32)
    else {
        debug!("Form effects skipped: the skin names no resource resolver");
        return Ok(None);
    };
    let Some(resolver_at) = file.entries.iter().position(|e| e.key_hash == resolver_key) else {
        warn!(
            resolver = format!("{resolver_key:08x}"),
            "Form effects skipped: the resource resolver is not in the skin bin"
        );
        return Ok(None);
    };
    let mut resolver = tree::parse_fields(&file.entries[resolver_at].body).map_err(bin_error)?;
    let base = resource_map(&embed("ResourceResolver", resolver.clone()));

    let keys: BTreeSet<u32> = gears.iter().flat_map(|g| g.keys().copied()).collect();
    let targets: BTreeMap<u32, Vec<Option<u32>>> = keys
        .iter()
        .map(|k| {
            let per_form = gears
                .iter()
                .map(|g| g.get(k).or(base.get(k)).copied().filter(|p| *p != 0))
                .collect::<Vec<_>>();
            (*k, per_form)
        })
        .filter(|(_, t)| t.iter().any(|p| *p != t[0]))
        .collect();
    let mut gate = Gate {
        systems: &systems,
        targets: &targets,
        added: BTreeMap::new(),
        stamped: 0,
        unresolved: 0,
    };
    let mut links = Vec::new();
    for (key, per_form) in &targets {
        if let Some((path, merged)) = gate.merge(*key, per_form) {
            gate.added.insert(path, merged);
            links.push((*key, path));
        }
    }
    if gate.unresolved > 0 {
        warn!(
            unresolved = gate.unresolved,
            "Child effects not found in the skin's bins; they stay ungated and show in every form"
        );
    }
    if links.is_empty() {
        debug!("Form effects skipped: no effect differs between forms");
        return Ok(None);
    }
    for (key, path, system) in marker_systems(forms) {
        gate.added.insert(path, system);
        links.push((key, path));
    }

    let Some(Value::Map { entries, .. }) = tree::field_mut(&mut resolver, h("resourceMap")) else {
        warn!("Form effects skipped: the resource resolver has no resource map");
        return Ok(None);
    };
    for (key, path) in links {
        let link = raw(FIELD_LINK, path.to_le_bytes().to_vec());
        match entries.iter_mut().find(|(k, _)| k.as_u32() == Some(key)) {
            Some(entry) => entry.1 = link,
            None => entries.push((hash_value(key), link)),
        }
    }
    file.entries[resolver_at].body = tree::write_fields(&resolver).map_err(bin_error)?;

    hold_markers(&mut skin, forms, plan.markers, plan.bone);
    file.entries[skin_at].body = tree::write_fields(&skin).map_err(bin_error)?;

    let stamped = gate.stamped;
    for (path, fields) in gate.added {
        file.entries.push(PropEntry {
            class_hash: h("VfxSystemDefinitionData"),
            key_hash: path,
            body: tree::write_fields(&fields).map_err(bin_error)?,
        });
    }
    serialize_prop_file(&file)
        .map(|bytes| Some((bytes, stamped)))
        .map_err(bin_error)
}

#[cfg(test)]
mod tests;
