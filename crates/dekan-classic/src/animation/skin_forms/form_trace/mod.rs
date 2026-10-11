use std::collections::{BTreeMap, BTreeSet, HashMap};

use dekan_wad::WadFile;
use dekan_wad::hash::{prop_key_hash as h, wad_path_hash};
use dekan_wad::prop::parse_prop_file;
use dekan_wad::prop::tree::{self, Value};
use tracing::{Level, debug};

use crate::gear_toggle::GearSwap;

const FIELD_STRING: u8 = 16;
const SKN_MAGIC: [u8; 4] = [0x33, 0x22, 0x11, 0x00];
const SKN_PART_SIZE: usize = 80;
const SKN_PART_NAME: usize = 64;
const SKN_PARTS_AT: usize = 12;
const MAX_DETAILED_CLIPS: usize = 64;
const CLIP_REF_FIELDS: [&str; 5] = [
    "mTrueConditionClipName",
    "mFalseConditionClipName",
    "mClipName",
    "mNextClipName",
    "mEndClipName",
];

fn texts(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Raw { kind, bytes } if *kind == FIELD_STRING && bytes.len() > 2 => {
            out.push(String::from_utf8_lossy(&bytes[2..]).into_owned());
        }
        Value::Raw { .. } | Value::Optional { value: None, .. } => {}
        Value::List { items, .. } => items.iter().for_each(|item| texts(item, out)),
        Value::Struct { fields, .. } => fields.iter().for_each(|f| texts(&f.value, out)),
        Value::Optional {
            value: Some(inner), ..
        } => texts(inner, out),
        Value::Map { entries, .. } => entries.iter().for_each(|(_, v)| texts(v, out)),
    }
}

fn skn_parts(bytes: &[u8]) -> Vec<String> {
    if bytes.get(..4) != Some(SKN_MAGIC.as_slice()) {
        return Vec::new();
    }
    let Some(count) = bytes
        .get(8..12)
        .and_then(|raw| raw.try_into().ok())
        .map(u32::from_le_bytes)
    else {
        return Vec::new();
    };
    (0..count as usize)
        .map_while(|i| {
            let at = SKN_PARTS_AT + i * SKN_PART_SIZE;
            let raw = bytes.get(at..at + SKN_PART_NAME)?;
            let end = raw.iter().position(|b| *b == 0).unwrap_or(SKN_PART_NAME);
            Some(String::from_utf8_lossy(&raw[..end]).into_owned())
        })
        .collect()
}

fn part_names(wad: &WadFile, skin_bin: &[u8]) -> HashMap<u32, String> {
    let mut found = Vec::new();
    if let Ok(prop) = parse_prop_file(skin_bin) {
        for entry in &prop.entries {
            if let Ok(fields) = tree::parse_fields(&entry.body) {
                fields.iter().for_each(|f| texts(&f.value, &mut found));
            }
        }
    }
    let mut names = HashMap::new();
    for mesh in found
        .iter()
        .filter(|t| t.to_ascii_lowercase().ends_with(".skn"))
        .collect::<BTreeSet<_>>()
    {
        if let Ok(Some(bytes)) = wad.read(wad_path_hash(&mesh.to_ascii_lowercase())) {
            for part in skn_parts(&bytes) {
                names.insert(h(&part), part);
            }
        }
    }
    names
}

fn hashes_of(fields: &[tree::Field], name: &str) -> Vec<u32> {
    tree::field(fields, h(name))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default()
}

fn clip_refs(value: &Value, out: &mut Vec<u32>) {
    match value {
        Value::Struct { fields, .. } => {
            for field in fields {
                if CLIP_REF_FIELDS.iter().any(|name| h(name) == field.name) {
                    out.extend(field.value.as_u32());
                } else if field.name == h("mClipNameList") {
                    out.extend(hashes_of(std::slice::from_ref(field), "mClipNameList"));
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

fn driven_parts(value: &Value, out: &mut Vec<u32>) {
    match value {
        Value::Struct { fields, class, .. } => {
            if *class == h("SubmeshVisibilityBoolDriver") {
                out.extend(fields.iter().filter_map(|f| f.value.as_u32()));
            }
            fields.iter().for_each(|f| driven_parts(&f.value, out));
        }
        Value::List { items, .. } => items.iter().for_each(|item| driven_parts(item, out)),
        Value::Optional {
            value: Some(inner), ..
        } => driven_parts(inner, out),
        Value::Map { entries, .. } => entries.iter().for_each(|(_, v)| driven_parts(v, out)),
        Value::Raw { .. } | Value::Optional { value: None, .. } => {}
    }
}

fn clips(graph_bin: &[u8], graph_key: u32) -> BTreeMap<u32, Value> {
    let Ok(file) = parse_prop_file(graph_bin) else {
        return BTreeMap::new();
    };
    let Some(entry) = file.entries.iter().find(|e| e.key_hash == graph_key) else {
        return BTreeMap::new();
    };
    let Ok(fields) = tree::parse_fields(&entry.body) else {
        return BTreeMap::new();
    };
    match tree::field(&fields, h("mClipDataMap")) {
        Some(Value::Map { entries, .. }) => entries
            .iter()
            .filter_map(|(k, v)| Some((k.as_u32()?, v.clone())))
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn visibility_events(clip: &Value) -> Vec<(Vec<u32>, Vec<u32>)> {
    let Some(Value::Map { entries, .. }) = clip
        .fields()
        .and_then(|fields| tree::field(fields, h("mEventDataMap")))
    else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|(_, event)| event.class() == Some(h("SubmeshVisibilityEventData")))
        .filter_map(|(_, event)| event.fields())
        .map(|ev| {
            (
                hashes_of(ev, "mShowSubmeshList"),
                hashes_of(ev, "mHideSubmeshList"),
            )
        })
        .collect()
}

pub struct FormTrace<'a> {
    pub wad: &'a WadFile,
    pub alias: &'a str,
    pub skin_bin: &'a [u8],
    pub graph_path: &'a str,
    pub graph_key: u32,
    pub generated_graph: &'a [u8],
    pub swaps: &'a [GearSwap],
    pub markers: &'a [u32],
}

pub fn trace(form: &FormTrace) {
    if !tracing::enabled!(Level::DEBUG) {
        return;
    }
    let names = part_names(form.wad, form.skin_bin);
    let name = |hash: &u32| {
        names
            .get(hash)
            .cloned()
            .unwrap_or_else(|| format!("{hash:08x}"))
    };
    let named = |list: &[u32]| list.iter().map(name).collect::<Vec<_>>();
    debug!(
        champion = form.alias,
        graph = form.graph_path,
        forms = form.swaps.len(),
        mesh_parts = names.len(),
        "Form trace: the skin's forms as the game defines them"
    );
    for (index, (swap, marker)) in form.swaps.iter().zip(form.markers).enumerate() {
        debug!(
            form = index,
            equip_clip = ?swap.equip,
            marker = %name(marker),
            show = ?named(&swap.show),
            hide = ?named(&swap.hide),
            "Form trace: form"
        );
    }

    let original = form
        .wad
        .read(wad_path_hash(form.graph_path))
        .ok()
        .flatten()
        .map(|bytes| clips(&bytes, form.graph_key))
        .unwrap_or_default();
    for (key, clip) in &original {
        for (show, hide) in visibility_events(clip) {
            debug!(
                clip = %format!("{key:08x}"),
                show = ?named(&show),
                hide = ?named(&hide),
                "Form trace: a game clip changes part visibility"
            );
        }
    }

    let generated = clips(form.generated_graph, form.graph_key);
    let added: Vec<u32> = generated
        .keys()
        .filter(|key| !original.contains_key(key))
        .copied()
        .collect();
    debug!(
        game_clips = original.len(),
        generated_clips = generated.len(),
        added = added.len(),
        "Form trace: clips added for the in-game form cycle"
    );
    let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
    for key in &added {
        let class = generated
            .get(key)
            .and_then(Value::class)
            .map_or_else(|| "unknown".to_owned(), |c| format!("{c:08x}"));
        *by_class.entry(class).or_default() += 1;
    }
    debug!(by_class = ?by_class, "Form trace: added clips by class hash");
    for key in added.into_iter().take(MAX_DETAILED_CLIPS) {
        let Some(clip) = generated.get(&key) else {
            continue;
        };
        let mut refs = Vec::new();
        clip_refs(clip, &mut refs);
        let events: Vec<String> = visibility_events(clip)
            .iter()
            .map(|(show, hide)| format!("show {:?} hide {:?}", named(show), named(hide)))
            .collect();
        let mut watched = Vec::new();
        driven_parts(clip, &mut watched);
        let drivers = named(&watched);
        debug!(
            clip = %format!("{key:08x}"),
            refs = ?refs.iter().map(|r| format!("{r:08x}")).collect::<Vec<_>>(),
            events = ?events,
            marker_drivers = ?drivers,
            "Form trace: added clip"
        );
    }
}

#[cfg(test)]
mod tests;
