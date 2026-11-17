use std::collections::BTreeMap;
use std::path::Path;

use dekan_wad::prop::FIELD_STRING;
use dekan_wad::prop::tree::{self, Field, Value};

use crate::error::ClassicError;
use crate::gear_toggle::h;
use crate::vfx_markers::{raw_text, text};

const FIELD_LIST: u8 = 0x80;

pub(super) fn base_tag_for(tags: &[String], character: &str) -> String {
    let lower_char = character.to_ascii_lowercase();
    let plain = lower_char.strip_prefix("jade_").unwrap_or(&lower_char);
    for tag in tags {
        if tag.eq_ignore_ascii_case(character) || tag.eq_ignore_ascii_case(plain) {
            return tag.clone();
        }
    }
    plain.to_owned()
}

pub(super) fn tag_prefix_end(name: &str) -> Option<usize> {
    let first = name.find('_')?;
    let rest = &name[first + 1..];
    let second = rest.find('_')?;
    let action = &name[..first];
    let category = &rest[..second];
    if action.is_empty()
        || !action.bytes().all(|b| b.is_ascii_alphabetic())
        || (!category.eq_ignore_ascii_case("sfx") && !category.eq_ignore_ascii_case("vo"))
    {
        return None;
    }
    Some(first + 1 + second + 1)
}

pub(super) fn write_bank_asset(
    wad_root: &Path,
    rel_path: &str,
    bytes: &[u8],
    shipped: &mut BTreeMap<String, String>,
) -> Result<(), ClassicError> {
    let key = rel_path.to_ascii_lowercase();
    let dst = wad_root.join(&key);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dst, bytes)?;
    shipped.insert(key.clone(), key);
    Ok(())
}

#[derive(Clone)]
pub(super) struct UnitEdit {
    pub unlist: Vec<String>,
    pub list: Vec<String>,
    pub bank_paths: Vec<String>,
}

pub(super) struct UnitSnapshot {
    pub events_bank: Option<String>,
    pub events: Vec<String>,
}

pub(super) fn string_items(fields: &[Field], field_name: &str) -> Vec<String> {
    tree::field(fields, h(field_name))
        .and_then(Value::items)
        .unwrap_or_default()
        .iter()
        .filter_map(raw_text)
        .collect()
}

pub(super) fn set_string_list(fields: &mut Vec<Field>, field_name: &str, items: &[String]) {
    tree::set_field(
        fields,
        h(field_name),
        Value::List {
            kind: FIELD_LIST,
            element: FIELD_STRING,
            items: items.iter().map(|s| text(s)).collect(),
        },
    );
}

fn append_unique_strings(unit: &mut Vec<Field>, field_name: &str, items: &[String]) {
    let Some(Value::List { items: target, .. }) = tree::field_mut(unit, h(field_name)) else {
        if !items.is_empty() {
            set_string_list(unit, field_name, items);
        }
        return;
    };
    for item in items {
        let exists = target
            .iter()
            .filter_map(raw_text)
            .any(|existing| existing.eq_ignore_ascii_case(item));
        if !exists {
            target.push(text(item));
        }
    }
}

fn base_unit_name(name: &str) -> Option<String> {
    let is_skin = |seg: &str| {
        seg.len() > 4
            && seg[..4].eq_ignore_ascii_case("skin")
            && seg[4..].bytes().all(|b| b.is_ascii_digit())
    };
    let segments: Vec<&str> = name.split('_').collect();
    if !segments.iter().any(|seg| is_skin(seg)) {
        return None;
    }
    let renamed: Vec<&str> = segments
        .into_iter()
        .map(|seg| if is_skin(seg) { "Base" } else { seg })
        .collect();
    Some(renamed.join("_"))
}

pub(super) fn rebase_bank_units(audio: &mut [Field]) -> bool {
    let Some(Value::List { items, .. }) = tree::field_mut(audio, h("bankUnits")) else {
        return false;
    };
    let mut changed = false;
    let mut kept: Vec<Value> = Vec::with_capacity(items.len());
    for mut item in std::mem::take(items) {
        let old_name = item
            .fields()
            .and_then(|f| tree::field(f, h("name")))
            .and_then(raw_text);
        let Some(new_name) = old_name.as_deref().and_then(base_unit_name) else {
            kept.push(item);
            continue;
        };
        changed = true;
        if let Some(uf) = item.fields_mut() {
            tree::set_field(uf, h("name"), text(&new_name));
        }
        let target = kept.iter_mut().find(|existing| {
            existing
                .fields()
                .and_then(|f| tree::field(f, h("name")))
                .and_then(raw_text)
                .is_some_and(|n| n.eq_ignore_ascii_case(&new_name))
        });
        if let Some(existing) = target {
            let paths = item
                .fields()
                .map(|f| string_items(f, "bankPath"))
                .unwrap_or_default();
            let events = item
                .fields()
                .map(|f| string_items(f, "events"))
                .unwrap_or_default();
            if let Some(ef) = existing.fields_mut() {
                append_unique_strings(ef, "bankPath", &paths);
                append_unique_strings(ef, "events", &events);
            }
        } else {
            kept.push(item);
        }
    }
    *items = kept;
    changed
}

pub(super) fn snapshot_units(audio: &[Field]) -> Vec<UnitSnapshot> {
    let Some(units_val) = tree::field(audio, h("bankUnits")).and_then(Value::items) else {
        return Vec::new();
    };
    units_val
        .iter()
        .map(|u| {
            let f = u.fields().unwrap_or(&[]);
            let paths = string_items(f, "bankPath");
            let events_bank = paths
                .into_iter()
                .find(|p| p.to_ascii_lowercase().ends_with("_events.bnk"));
            UnitSnapshot {
                events_bank,
                events: string_items(f, "events"),
            }
        })
        .collect()
}

pub(super) fn apply_unit_edits(
    unit_values: &mut [Value],
    snapshots: &[UnitSnapshot],
    edits: &BTreeMap<usize, UnitEdit>,
) {
    for (&idx, edit) in edits {
        if let Some(Value::List { items, .. }) = unit_values
            .get_mut(idx)
            .and_then(Value::fields_mut)
            .and_then(|uf| tree::field_mut(uf, h("events")))
        {
            items.retain(|v| {
                !raw_text(v).is_some_and(|n| edit.unlist.iter().any(|u| u.eq_ignore_ascii_case(&n)))
            });
        }
    }
    for (&idx, edit) in edits {
        let Some(uf) = unit_values.get_mut(idx).and_then(Value::fields_mut) else {
            continue;
        };
        append_unique_strings(uf, "events", &edit.list);
        if !edit.bank_paths.is_empty() {
            if let Some(Value::List { items, .. }) = tree::field_mut(uf, h("bankPath")) {
                let events_bank = snapshots.get(idx).and_then(|s| s.events_bank.as_deref());
                for path in &edit.bank_paths {
                    if items
                        .iter()
                        .filter_map(raw_text)
                        .any(|p| p.eq_ignore_ascii_case(path))
                    {
                        continue;
                    }
                    let insert_at = events_bank.and_then(|eb| {
                        items
                            .iter()
                            .position(|v| raw_text(v).is_some_and(|p| p.eq_ignore_ascii_case(eb)))
                    });
                    match insert_at {
                        Some(pos) => items.insert(pos + 1, text(path)),
                        None => items.push(text(path)),
                    }
                }
            }
        }
    }
}
