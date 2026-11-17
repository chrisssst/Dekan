use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::wad::WadFile;

use super::banks::{LoadedBank, load_bank, sidecar_path};
use super::units::{
    UnitEdit, UnitSnapshot, apply_unit_edits, base_tag_for, rebase_bank_units, set_string_list,
    snapshot_units, string_items, tag_prefix_end, write_bank_asset,
};
use crate::error::ClassicError;
use crate::gear_toggle::h;
use crate::sound_bank::wwise_id;

struct Candidate {
    priority: isize,
    is_base: bool,
    unit: usize,
    name: String,
    prefix_end: usize,
    suffix_start: usize,
    listed: bool,
}

type SidecarMap = BTreeMap<String, (String, String, Vec<(u32, u32)>)>;

pub(super) fn rebase_skin_audio(
    wad: &WadFile,
    character: &str,
    fields: &mut [Field],
    game_base_tags: Option<&[String]>,
    wad_root: &Path,
    shipped: &mut BTreeMap<String, String>,
) -> Result<bool, ClassicError> {
    let Some(Value::Struct { fields: audio, .. }) =
        tree::field_mut(fields, h("skinAudioProperties"))
    else {
        return Ok(false);
    };

    let current_tags = string_items(audio, "tagEventList");
    let base_tags: Vec<String> = match game_base_tags {
        Some(list) if list.iter().any(|t| !t.is_empty()) => list.to_vec(),
        _ => vec![base_tag_for(&current_tags, character)],
    };

    let mut changed = current_tags != base_tags;

    let base_tag = base_tags.iter().rev().find(|t| !t.is_empty()).cloned();
    let is_base_tag = |tag: &str| base_tags.iter().any(|b| b.eq_ignore_ascii_case(tag));
    let removed_tags: Vec<String> = current_tags
        .iter()
        .filter(|t| !t.is_empty() && !is_base_tag(t))
        .cloned()
        .collect();

    if let Some(ref base_tag) = base_tag {
        if !removed_tags.is_empty()
            && merge_banks(
                wad,
                wad_root,
                audio,
                &current_tags,
                &base_tags,
                base_tag,
                &removed_tags,
                shipped,
            )?
        {
            changed = true;
        }
    }

    if rebase_bank_units(audio) {
        changed = true;
    }

    set_string_list(audio, "tagEventList", &base_tags);
    Ok(changed)
}

#[allow(clippy::too_many_arguments)]
fn merge_banks(
    wad: &WadFile,
    wad_root: &Path,
    audio: &mut [Field],
    tags: &[String],
    base_tags: &[String],
    base_tag: &str,
    removed_tags: &[String],
    shipped: &mut BTreeMap<String, String>,
) -> Result<bool, ClassicError> {
    let units = snapshot_units(audio);
    if units.is_empty() {
        return Ok(false);
    }

    let mut locale_wads = None;
    let mut loaded: BTreeMap<String, LoadedBank> = BTreeMap::new();
    let mut ordered_bank_paths = Vec::new();
    for u in &units {
        let Some(ref path) = u.events_bank else {
            continue;
        };
        let key = path.to_ascii_lowercase();
        if let std::collections::btree_map::Entry::Vacant(slot) = loaded.entry(key) {
            if let Some(lb) = load_bank(wad, wad_root, &mut locale_wads, path) {
                ordered_bank_paths.push(lb.path.clone());
                slot.insert(lb);
            }
        }
    }
    if loaded.is_empty() {
        return Ok(false);
    }

    let is_base = |tag: &str| base_tags.iter().any(|b| b.eq_ignore_ascii_case(tag));
    let priorities = build_priorities(tags, base_tags);
    let bank_events = collect_bank_events(&units, &loaded);
    let candidates = collect_candidates(&units, &priorities, &is_base);
    let posted: BTreeSet<u32> = units
        .iter()
        .flat_map(|u| u.events.iter())
        .map(|n| wwise_id(n))
        .collect();

    let mut sidecars: SidecarMap = BTreeMap::new();
    let mut edits: BTreeMap<usize, UnitEdit> = BTreeMap::new();
    let mut any_plan = false;

    for (_key, group) in candidates {
        let Some(winner) = group
            .iter()
            .filter(|c| bank_events.contains_key(&wwise_id(&c.name)))
            .max_by_key(|c| (c.priority, c.listed, c.unit))
        else {
            continue;
        };
        let prefix = &winner.name[..winner.prefix_end];
        let suffix = &winner.name[winner.suffix_start..];
        let aliases = plan_aliases(
            winner,
            prefix,
            suffix,
            base_tag,
            removed_tags,
            &bank_events,
            &posted,
        );
        if aliases.is_empty() {
            continue;
        }
        let source_id = wwise_id(&winner.name);
        let own_bank = units[winner.unit].events_bank.as_deref();
        let Some(source_bank) = own_bank
            .into_iter()
            .chain(ordered_bank_paths.iter().map(String::as_str))
            .find(|p| {
                loaded
                    .get(&p.to_ascii_lowercase())
                    .is_some_and(|lb| lb.bank.has_event(source_id))
            })
            .map(str::to_owned)
        else {
            continue;
        };
        let source_localized = loaded
            .get(&source_bank.to_ascii_lowercase())
            .is_some_and(|lb| lb.localized);

        for alias in aliases {
            let alias_id = wwise_id(&alias);
            for path in &ordered_bank_paths {
                if path.eq_ignore_ascii_case(&source_bank) {
                    continue;
                }
                let Some(lb) = loaded.get_mut(&path.to_ascii_lowercase()) else {
                    continue;
                };
                if lb.localized || !lb.bank.has_event(alias_id) {
                    continue;
                }
                if lb.bank.remove_event(alias_id) {
                    any_plan = true;
                }
                for (u_idx, u) in units.iter().enumerate() {
                    if u_idx != winner.unit
                        && u.events_bank
                            .as_deref()
                            .is_some_and(|eb| eb.eq_ignore_ascii_case(path))
                    {
                        edit_of(&mut edits, u_idx).unlist.push(alias.clone());
                    }
                }
            }
            if source_localized {
                let sc_path = sidecar_path(&source_bank);
                let entry = sidecars
                    .entry(source_bank.to_ascii_lowercase())
                    .or_insert_with(|| (source_bank.clone(), sc_path.clone(), Vec::new()));
                if !entry.2.iter().any(|&(a, _)| a == alias_id) {
                    entry.2.push((alias_id, source_id));
                }
                let ed = edit_of(&mut edits, winner.unit);
                if !ed
                    .bank_paths
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(&sc_path))
                {
                    ed.bank_paths.push(sc_path);
                }
            } else if let Some(src_lb) = loaded.get_mut(&source_bank.to_ascii_lowercase()) {
                if let Some(actions) = src_lb.bank.event_actions(source_id) {
                    src_lb.bank.set_or_add_event(alias_id, &actions);
                    any_plan = true;
                }
            }
            let ed = edit_of(&mut edits, winner.unit);
            if !ed.list.iter().any(|e| e.eq_ignore_ascii_case(&alias)) {
                ed.list.push(alias);
            }
        }
    }

    for (_k, (source_path, sc_path, pairs)) in sidecars {
        if let Some(src_lb) = loaded.get(&source_path.to_ascii_lowercase()) {
            let stem = Path::new(&sc_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&sc_path);
            if let Ok(bytes) = src_lb.bank.extract_sidecar(wwise_id(stem), &pairs) {
                write_bank_asset(wad_root, &sc_path, &bytes, shipped)?;
                any_plan = true;
            }
        }
    }
    for lb in loaded.values() {
        if lb.bank.modified {
            write_bank_asset(wad_root, &lb.path, &lb.bank.write()?, shipped)?;
            any_plan = true;
        }
    }
    if !edits.is_empty() {
        if let Some(Value::List {
            items: unit_vals, ..
        }) = tree::field_mut(audio, h("bankUnits"))
        {
            apply_unit_edits(unit_vals, &units, &edits);
            any_plan = true;
        }
    }
    Ok(any_plan)
}

fn edit_of(edits: &mut BTreeMap<usize, UnitEdit>, idx: usize) -> &mut UnitEdit {
    edits.entry(idx).or_insert_with(|| UnitEdit {
        unlist: Vec::new(),
        list: Vec::new(),
        bank_paths: Vec::new(),
    })
}

fn build_priorities(tags: &[String], base_tags: &[String]) -> Vec<(String, isize)> {
    let mut priorities: Vec<(String, isize)> = tags
        .iter()
        .filter(|t| !t.is_empty())
        .enumerate()
        .map(|(i, t)| (t.clone(), i as isize))
        .collect();
    let unconsulted: Vec<String> = base_tags
        .iter()
        .filter(|b| !b.is_empty() && !tags.iter().any(|t| t.eq_ignore_ascii_case(b)))
        .cloned()
        .collect();
    for (i, tag) in unconsulted.iter().enumerate() {
        priorities.push((tag.clone(), i as isize - unconsulted.len() as isize));
    }
    priorities
}

fn collect_bank_events(
    units: &[UnitSnapshot],
    loaded: &BTreeMap<String, LoadedBank>,
) -> BTreeMap<u32, Vec<usize>> {
    let mut bank_events: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (unit_idx, u) in units.iter().enumerate() {
        let Some(ref bp) = u.events_bank else {
            continue;
        };
        if let Some(lb) = loaded.get(&bp.to_ascii_lowercase()) {
            for eid in lb.bank.event_ids() {
                let entry = bank_events.entry(eid).or_default();
                if !entry.contains(&unit_idx) {
                    entry.push(unit_idx);
                }
            }
        }
    }
    bank_events
}

fn collect_candidates(
    units: &[UnitSnapshot],
    priorities: &[(String, isize)],
    is_base: &dyn Fn(&str) -> bool,
) -> BTreeMap<(String, String), Vec<Candidate>> {
    let mut candidates: BTreeMap<(String, String), Vec<Candidate>> = BTreeMap::new();
    for (unit_idx, u) in units.iter().enumerate() {
        for name in &u.events {
            let Some(pe) = tag_prefix_end(name) else {
                continue;
            };
            let rest = &name[pe..];
            for (tag, priority) in priorities {
                if rest.len() <= tag.len()
                    || !rest[..tag.len()].eq_ignore_ascii_case(tag)
                    || !rest[tag.len()..].starts_with('_')
                {
                    continue;
                }
                let ss = pe + tag.len() + 1;
                let key = (
                    name[..pe].to_ascii_lowercase(),
                    name[ss..].to_ascii_lowercase(),
                );
                candidates.entry(key).or_default().push(Candidate {
                    priority: *priority,
                    is_base: is_base(tag),
                    unit: unit_idx,
                    name: name.clone(),
                    prefix_end: pe,
                    suffix_start: ss,
                    listed: true,
                });
            }
        }
    }
    candidates
}

fn plan_aliases(
    winner: &Candidate,
    prefix: &str,
    suffix: &str,
    base_tag: &str,
    removed_tags: &[String],
    bank_events: &BTreeMap<u32, Vec<usize>>,
    posted: &BTreeSet<u32>,
) -> Vec<String> {
    let mut aliases = Vec::new();
    if !winner.is_base {
        aliases.push(format!("{prefix}{base_tag}_{suffix}"));
    }
    for dtag in removed_tags {
        let alias = format!("{prefix}{dtag}_{suffix}");
        let aid = wwise_id(&alias);
        if alias.eq_ignore_ascii_case(&winner.name)
            || aliases.iter().any(|a| a.eq_ignore_ascii_case(&alias))
            || bank_events.contains_key(&aid)
        {
            continue;
        }
        if winner.is_base && !posted.contains(&aid) {
            continue;
        }
        aliases.push(alias);
    }
    aliases
}
