use std::collections::BTreeSet;
use std::path::Path;

use dekan_classic::forms::skn_submesh_names;
use dekan_classic::gear_toggle::gear_swap;
use dekan_classic::generator::StandardChampion;
use dekan_wad::hash::prop_key_hash as h;
use dekan_wad::prop::parse_prop_file;
use dekan_wad::prop::tree::{self, Value};

use super::SkinFinding;

const SKIN_DATA_CLASS: u32 = 0x9b67_e9f6;

fn split_submesh_list(raw: &str) -> BTreeSet<u32> {
    raw.split(|c: char| c.is_whitespace() || matches!(c, ',' | '|' | ':'))
        .filter(|s| !s.is_empty())
        .map(h)
        .collect()
}

fn is_dekan_form_copy(sub: &str) -> bool {
    let Some((_, tail)) = sub.rsplit_once("_F") else {
        return false;
    };
    !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())
}

fn raw_text(value: &Value) -> Option<String> {
    match value {
        Value::Raw { kind: 16, bytes } if bytes.len() >= 2 => {
            Some(String::from_utf8_lossy(&bytes[2..]).into_owned())
        }
        _ => None,
    }
}

fn hashes_of(fields: &[tree::Field], name: &str) -> Vec<u32> {
    tree::field(fields, h(name))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default()
}

fn skin_fields_of(bytes: &[u8]) -> Option<Vec<tree::Field>> {
    let file = parse_prop_file(bytes).ok()?;
    let entry = file
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)?;
    tree::parse_fields(&entry.body).ok()
}

fn anim_graph_key(fields: &[tree::Field]) -> Option<u32> {
    tree::field(fields, h("skinAnimationProperties"))
        .and_then(Value::fields)
        .and_then(|a| tree::field(a, h("animationGraphData")))
        .and_then(Value::as_u32)
}

fn mesh_field_text(fields: &[tree::Field], name: &str) -> Option<String> {
    tree::field(fields, h("skinMeshProperties"))
        .and_then(Value::fields)
        .and_then(|m| tree::field(m, h(name)))
        .and_then(raw_text)
}

fn material_override_submeshes(fields: &[tree::Field]) -> Vec<String> {
    tree::field(fields, h("skinMeshProperties"))
        .and_then(Value::fields)
        .and_then(|m| tree::field(m, h("materialOverride")))
        .and_then(Value::items)
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.fields().and_then(|f| tree::field(f, h("submesh"))))
        .filter_map(raw_text)
        .collect()
}

fn form0_hidden_in_conditions(fields: &[tree::Field]) -> BTreeSet<u32> {
    let mut hidden = BTreeSet::new();
    let Some(items) = tree::field(fields, h("PersistentEffectConditions")).and_then(Value::items)
    else {
        return hidden;
    };
    for cond in items {
        let Some(cf) = cond.fields() else {
            continue;
        };
        let is_form0 = tree::field(cf, h("OwnerCondition"))
            .and_then(Value::class)
            .is_some_and(|c| c == h("NotMaterialDriver"));
        if is_form0 {
            hidden.extend(hashes_of(cf, "SubmeshesToHide"));
        }
    }
    hidden
}

fn form0_hidden_in_graph(graph_bytes: &[u8], graph_key: u32) -> BTreeSet<u32> {
    let mut hidden = BTreeSet::new();
    let Ok(file) = parse_prop_file(graph_bytes) else {
        return hidden;
    };
    let Some(entry) = file.entries.iter().find(|e| e.key_hash == graph_key) else {
        return hidden;
    };
    let Ok(fields) = tree::parse_fields(&entry.body) else {
        return hidden;
    };
    let Some(Value::Map { entries, .. }) = tree::field(&fields, h("mClipDataMap")) else {
        return hidden;
    };
    let swap0 = dekan_classic::form_state::swap_key(0);
    let state0 = dekan_classic::form_state::state_key(0);
    for (k, clip) in entries {
        let Some(key) = k.as_u32() else {
            continue;
        };
        if key != state0 && key != swap0 {
            continue;
        }
        let Some(Value::Map {
            entries: events, ..
        }) = clip
            .fields()
            .and_then(|f| tree::field(f, h("mEventDataMap")))
        else {
            continue;
        };
        for (_, ev) in events {
            if ev.class() == Some(h("SubmeshVisibilityEventData"))
                && let Some(ef) = ev.fields()
            {
                hidden.extend(hashes_of(ef, "mHideSubmeshList"));
            }
        }
    }
    hidden
}

pub(super) fn check_skin_fidelity(
    champion: &StandardChampion,
    main: &str,
    skin: u32,
    wad_root: &Path,
    finding: &mut SkinFinding,
) {
    let Ok(Some(source_bytes)) = champion.read_skin_bin(main, skin) else {
        return;
    };
    let gen_path = wad_root
        .join("data")
        .join("characters")
        .join(main)
        .join("skins")
        .join("skin0.bin");
    let Ok(gen_bytes) = std::fs::read(&gen_path) else {
        return;
    };
    let (Some(src_fields), Some(gen_fields)) =
        (skin_fields_of(&source_bytes), skin_fields_of(&gen_bytes))
    else {
        return;
    };

    let src_graph = anim_graph_key(&src_fields);
    let gen_graph = anim_graph_key(&gen_fields);
    if src_graph.is_some() && src_graph != gen_graph {
        finding.stale_references.push(format!(
            "{main} skin{skin}: animationGraphData drifted from {src_graph:?} to {gen_graph:?}"
        ));
    }

    super::asset_checks::check_deep_assets(
        champion,
        main,
        skin,
        wad_root,
        &gen_bytes,
        &gen_fields,
        gen_graph,
        finding,
    );

    let src_skn_rel = mesh_field_text(&src_fields, "simpleSkin").map(|s| s.to_ascii_lowercase());
    let src_skn_parts: Vec<String> = src_skn_rel
        .as_deref()
        .and_then(|rel| champion.read_path(rel).ok().flatten())
        .and_then(|b| skn_submesh_names(&b).ok())
        .unwrap_or_default();

    let gen_skn_rel = mesh_field_text(&gen_fields, "simpleSkin").map(|s| s.to_ascii_lowercase());
    let skn_bytes = gen_skn_rel.as_deref().and_then(|rel| {
        let local = wad_root.join(rel);
        std::fs::read(local)
            .ok()
            .or_else(|| champion.read_path(rel).ok().flatten())
    });
    let skn_parts: Vec<String> = skn_bytes
        .as_deref()
        .and_then(|b| skn_submesh_names(b).ok())
        .unwrap_or_default();
    let skn_hashes: BTreeSet<u32> = skn_parts.iter().map(|n| h(n)).collect();

    if !skn_parts.is_empty() {
        for sub in material_override_submeshes(&gen_fields) {
            if is_dekan_form_copy(&sub) && !skn_hashes.contains(&h(&sub)) {
                finding.stale_references.push(format!(
                    "{main} skin{skin}: materialOverride references missing form copy '{sub}'"
                ));
            }
        }
    }

    if let Some(skl_rel) = mesh_field_text(&gen_fields, "skeleton") {
        let local = wad_root.join(skl_rel.to_ascii_lowercase());
        if local.is_file()
            && let Ok(bytes) = std::fs::read(&local)
            && let Err(e) = dekan_classic::skeleton::parse(&bytes)
        {
            finding
                .stale_references
                .push(format!("{main} skin{skin}: rebuilt skeleton invalid: {e}"));
        }
    }

    let Ok(gear_keys) = dekan_classic::forms::gear_keys(&source_bytes) else {
        return;
    };
    if gear_keys.len() < 2 || champion.gear_count(0) > 0 || skn_parts.is_empty() {
        return;
    }

    let src_init_hide = mesh_field_text(&src_fields, "initialSubmeshToHide")
        .map(|s| split_submesh_list(&s))
        .unwrap_or_default();
    let gear0_hide: BTreeSet<u32> = champion
        .gear_body(&source_bytes, gear_keys[0])
        .ok()
        .and_then(|body| gear_swap(&body).ok())
        .map(|swap| swap.hide.into_iter().collect())
        .unwrap_or_default();

    let src_cond_hide: BTreeSet<u32> = tree::field(&src_fields, h("PersistentEffectConditions"))
        .and_then(Value::items)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::fields)
        .flat_map(|cf| hashes_of(cf, "SubmeshesToHide"))
        .collect();
    let gen_init_hide = mesh_field_text(&gen_fields, "initialSubmeshToHide")
        .map(|s| split_submesh_list(&s))
        .unwrap_or_default();
    let cond_hide = form0_hidden_in_conditions(&gen_fields);

    let mut graph_hide = BTreeSet::new();
    if let Some(gk) = gen_graph
        && let Ok(parsed) = parse_prop_file(&gen_bytes)
    {
        for link in &parsed.links {
            let lower = link.to_ascii_lowercase();
            if lower.contains("/animations/") {
                let local = wad_root.join(&lower);
                if local.is_file()
                    && let Ok(bytes) = std::fs::read(&local)
                {
                    graph_hide.extend(form0_hidden_in_graph(&bytes, gk));
                }
            }
        }
    }

    for part in &src_skn_parts {
        let lower = part.to_ascii_lowercase();
        if lower.starts_with("dekanform") || is_dekan_form_copy(part) {
            continue;
        }
        let ph = h(part);
        let default_visible = !src_init_hide.contains(&ph)
            && !gear0_hide.contains(&ph)
            && !src_cond_hide.contains(&ph);
        if !default_visible {
            continue;
        }
        if gen_init_hide.contains(&ph) || cond_hide.contains(&ph) || graph_hide.contains(&ph) {
            finding.stale_references.push(format!(
                "{main} skin{skin}: default-visible submesh '{part}' is wrongly hidden in Form 0 (init={}, cond={}, graph={})",
                gen_init_hide.contains(&ph),
                cond_hide.contains(&ph),
                graph_hide.contains(&ph),
            ));
        }
    }
}

pub(super) fn check_baked_form_fidelity(
    champion: &StandardChampion,
    main: &str,
    skin: u32,
    form: u32,
    wad_root: &Path,
    finding: &mut SkinFinding,
) {
    let gen_path = wad_root
        .join("data")
        .join("characters")
        .join(main)
        .join("skins")
        .join("skin0.bin");
    let Ok(gen_bytes) = std::fs::read(&gen_path) else {
        finding
            .stale_references
            .push(format!("{main} skin{skin} form{form}: missing skin0.bin"));
        return;
    };
    let Some(gen_fields) = skin_fields_of(&gen_bytes) else {
        finding.stale_references.push(format!(
            "{main} skin{skin} form{form}: unreadable SkinCharacterDataProperties"
        ));
        return;
    };
    let gen_graph = anim_graph_key(&gen_fields);
    super::asset_checks::check_deep_assets(
        champion,
        main,
        skin,
        wad_root,
        &gen_bytes,
        &gen_fields,
        gen_graph,
        finding,
    );
    if let Some(skn_rel) = mesh_field_text(&gen_fields, "simpleSkin") {
        let lower = skn_rel.to_ascii_lowercase();
        let bytes = std::fs::read(wad_root.join(&lower))
            .ok()
            .or_else(|| champion.read_path(&lower).ok().flatten());
        if let Some(b) = bytes {
            if let Ok(parts) = skn_submesh_names(&b) {
                let part_hashes: BTreeSet<u32> = parts.iter().map(|n| h(n)).collect();
                for sub in material_override_submeshes(&gen_fields) {
                    if is_dekan_form_copy(&sub) && !part_hashes.contains(&h(&sub)) {
                        finding.stale_references.push(format!(
                            "{main} skin{skin} form{form}: materialOverride submesh '{sub}' not in mesh"
                        ));
                    }
                }
            }
        } else {
            finding.missing_links.push(format!(
                "{main} skin{skin} form{form}: missing simpleSkin '{lower}'"
            ));
        }
    }
    if let Some(skl_rel) = mesh_field_text(&gen_fields, "skeleton") {
        let lower = skl_rel.to_ascii_lowercase();
        let bytes = std::fs::read(wad_root.join(&lower))
            .ok()
            .or_else(|| champion.read_path(&lower).ok().flatten());
        if bytes.is_none() {
            finding.missing_links.push(format!(
                "{main} skin{skin} form{form}: missing skeleton '{lower}'"
            ));
        }
    }
}
