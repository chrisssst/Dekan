use std::collections::BTreeSet;
use std::path::Path;

use dekan_classic::generator::StandardChampion;
use dekan_classic::loading_name::rst;
use dekan_wad::hash::prop_key_hash as h;
use dekan_wad::prop::parse_prop_file;
use dekan_wad::prop::tree::{self, Value};

use super::SkinFinding;
use super::checks::{has_skin_numbered_ref, valid_texture_bytes};

const VFX_SYSTEM_CLASS: u32 = 0x45cd_8998;
const STATIC_MATERIAL_CLASS: u32 = 0xff9d_3409;

fn raw_text(value: &Value) -> Option<String> {
    match value {
        Value::Raw { kind: 16, bytes } if bytes.len() >= 2 => {
            Some(String::from_utf8_lossy(&bytes[2..]).into_owned())
        }
        _ => None,
    }
}

fn check_anm_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && (bytes.starts_with(b"r3d2anmd") || bytes.starts_with(b"r3d2canm"))
}

fn check_audio_magic(path: &str, bytes: &[u8]) -> bool {
    if path.ends_with(".bnk") {
        bytes.starts_with(b"BKHD") && dekan_classic::sound_bank::parse(bytes).is_ok()
    } else if path.ends_with(".wpk") {
        bytes.len() >= 12 && bytes.starts_with(b"r3d2")
    } else if path.ends_with(".wem") {
        bytes.len() >= 12 && bytes.starts_with(b"RIFF")
    } else {
        !bytes.is_empty()
    }
}

fn collect_defined_keys(
    champion: &StandardChampion,
    wad_root: &Path,
    gen_bytes: &[u8],
) -> BTreeSet<u32> {
    let mut keys = BTreeSet::new();
    let Ok(file) = parse_prop_file(gen_bytes) else {
        return keys;
    };
    keys.extend(file.entries.iter().map(|e| e.key_hash));
    for link in &file.links {
        let lower = link.to_ascii_lowercase();
        let bytes = std::fs::read(wad_root.join(&lower))
            .ok()
            .or_else(|| champion.read_path(&lower).ok().flatten());
        if let Some(b) = bytes
            && let Ok(linked) = parse_prop_file(&b)
        {
            keys.extend(linked.entries.into_iter().map(|e| e.key_hash));
        }
    }
    keys
}

fn check_audio_banks_and_rebase(
    champion: &StandardChampion,
    main: &str,
    skin: u32,
    wad_root: &Path,
    gen_fields: &[tree::Field],
    finding: &mut SkinFinding,
) {
    if let Some(name) = tree::field(gen_fields, h("championSkinName")).and_then(raw_text)
        && has_skin_numbered_ref(&name, main)
    {
        finding.stale_references.push(format!(
            "{main} skin{skin}: un-rebased championSkinName '{name}'"
        ));
    }
    let Some(audio) = tree::field(gen_fields, h("skinAudioProperties")).and_then(Value::fields)
    else {
        return;
    };
    if let Some(tags) = tree::field(audio, h("tagEventList")).and_then(Value::items) {
        for tag in tags.iter().filter_map(raw_text) {
            if has_skin_numbered_ref(&tag, main) {
                finding.stale_references.push(format!(
                    "{main} skin{skin}: un-rebased tagEventList '{tag}'"
                ));
            }
        }
    }
    let Some(units) = tree::field(audio, h("bankUnits")).and_then(Value::items) else {
        return;
    };
    for uf in units.iter().filter_map(Value::fields) {
        if let Some(uname) = tree::field(uf, h("name")).and_then(raw_text)
            && has_skin_numbered_ref(&uname, main)
        {
            finding.stale_references.push(format!(
                "{main} skin{skin}: un-rebased bankUnits.name '{uname}'"
            ));
        }
        if let Some(events) = tree::field(uf, h("events")).and_then(Value::items)
            && let Some(ev) = events
                .iter()
                .filter_map(raw_text)
                .find(|ev| has_skin_numbered_ref(ev, main))
        {
            finding.stale_references.push(format!(
                "{main} skin{skin}: un-rebased bankUnits.events '{ev}'"
            ));
        }
        let Some(paths) = tree::field(uf, h("bankPath")).and_then(Value::items) else {
            continue;
        };
        for p in paths.iter().filter_map(raw_text) {
            let lower = p.to_ascii_lowercase();
            let local = wad_root.join(&lower);
            if local.is_file() {
                if let Ok(bytes) = std::fs::read(&local)
                    && !check_audio_magic(&lower, &bytes)
                {
                    finding.stale_references.push(format!(
                        "{main} skin{skin}: generated audio bank '{lower}' has invalid header magic"
                    ));
                }
            } else if !lower.contains("/en_us/")
                && !lower.contains("/vo/")
                && !champion.contains_path(&lower)
            {
                finding.missing_links.push(format!(
                    "{main} skin{skin}: missing audio bank '{lower}' in WAD"
                ));
            }
        }
    }
}

fn check_animation_clips_and_anm(
    champion: &StandardChampion,
    main: &str,
    skin: u32,
    wad_root: &Path,
    graph_bytes: &[u8],
    graph_key: u32,
    finding: &mut SkinFinding,
) {
    let Ok(file) = parse_prop_file(graph_bytes) else {
        finding.stale_references.push(format!(
            "{main} skin{skin}: generated animation graph is not a valid PROP"
        ));
        return;
    };
    let Some(entry) = file.entries.iter().find(|e| e.key_hash == graph_key) else {
        return;
    };
    let Ok(fields) = tree::parse_fields(&entry.body) else {
        return;
    };
    let map_keys = |name: &str| -> BTreeSet<u32> {
        match tree::field(&fields, h(name)) {
            Some(Value::Map { entries, .. }) => {
                entries.iter().filter_map(|(k, _)| k.as_u32()).collect()
            }
            _ => BTreeSet::new(),
        }
    };
    let (tracks, masks) = (map_keys("mTrackDataMap"), map_keys("mMaskDataMap"));
    let Some(Value::Map { entries: clips, .. }) = tree::field(&fields, h("mClipDataMap")) else {
        return;
    };
    let clip_keys: BTreeSet<u32> = clips.iter().filter_map(|(k, _)| k.as_u32()).collect();
    for cf in clips.iter().filter_map(|(_, c)| c.fields()) {
        if let Some(tk) = tree::field(cf, h("mTrackDataName")).and_then(Value::as_u32)
            && !tracks.is_empty()
            && !tracks.contains(&tk)
        {
            finding.stale_references.push(format!(
                "{main} skin{skin}: clip references undefined track {tk:08x}"
            ));
        }
        if let Some(mk) = tree::field(cf, h("mMaskDataName")).and_then(Value::as_u32)
            && !masks.is_empty()
            && !masks.contains(&mk)
        {
            finding.stale_references.push(format!(
                "{main} skin{skin}: clip references undefined mask {mk:08x}"
            ));
        }
        for ref_field in [
            "mTrueConditionClipName",
            "mFalseConditionClipName",
            "mClipName",
        ] {
            if let Some(target) = tree::field(cf, h(ref_field)).and_then(Value::as_u32)
                && target != 0
                && !clip_keys.contains(&target)
            {
                finding.stale_references.push(format!(
                    "{main} skin{skin}: clip field {ref_field} points to missing clip {target:08x}"
                ));
            }
        }
        if let Some(anm) = tree::field(cf, h("mAnimationResourceData"))
            .and_then(Value::fields)
            .and_then(|r| tree::field(r, h("mAnimationFilePath")))
            .and_then(raw_text)
        {
            let lower = anm.to_ascii_lowercase();
            if let Some(b) = std::fs::read(wad_root.join(&lower))
                .ok()
                .or_else(|| champion.read_path(&lower).ok().flatten())
                && !check_anm_magic(&b)
            {
                finding.stale_references.push(format!(
                    "{main} skin{skin}: animation '{lower}' has invalid .anm header"
                ));
            }
        }
    }
}

fn check_vfx_materials_and_character_record(
    champion: &StandardChampion,
    main: &str,
    skin: u32,
    wad_root: &Path,
    gen_bytes: &[u8],
    finding: &mut SkinFinding,
) {
    if let Ok(file) = parse_prop_file(gen_bytes) {
        for entry in &file.entries {
            if entry.class_hash == VFX_SYSTEM_CLASS && tree::parse_fields(&entry.body).is_err() {
                finding.stale_references.push(format!(
                    "{main} skin{skin}: VfxSystemDefinitionData {:08x} body corrupted",
                    entry.key_hash
                ));
            } else if entry.class_hash == STATIC_MATERIAL_CLASS {
                let Ok(mfields) = tree::parse_fields(&entry.body) else {
                    finding.stale_references.push(format!(
                        "{main} skin{skin}: StaticMaterialDef {:08x} body corrupted",
                        entry.key_hash
                    ));
                    continue;
                };
                if let Some(samplers) =
                    tree::field(&mfields, h("samplerValues")).and_then(Value::items)
                {
                    for s in samplers.iter().filter_map(Value::fields) {
                        if let Some(t) = tree::field(s, h("textureName"))
                            .or_else(|| tree::field(s, h("texturePath")))
                            .and_then(raw_text)
                        {
                            check_texture_ref(champion, main, skin, wad_root, &t, finding);
                        }
                    }
                }
            }
        }
    }
    let record_path = format!("data/characters/{main}/{main}.bin");
    if let Ok(Some(record_bytes)) = champion.read_path(&record_path)
        && let Some(patched) = rst::patch_record_key(&record_bytes, &champion.alias)
        && parse_prop_file(&patched).is_err()
    {
        finding.stale_references.push(format!(
            "{main} skin{skin}: loading_name CharacterRecord patch corrupted PROP layout"
        ));
    }
}

fn check_texture_ref(
    champion: &StandardChampion,
    main: &str,
    skin: u32,
    wad_root: &Path,
    rel: &str,
    finding: &mut SkinFinding,
) {
    let lower = rel.to_ascii_lowercase();
    if let Some(bytes) = std::fs::read(wad_root.join(&lower))
        .ok()
        .or_else(|| champion.read_path(&lower).ok().flatten())
        && !valid_texture_bytes(&bytes)
    {
        finding.stale_references.push(format!(
            "{main} skin{skin}: invalid texture header in '{lower}'"
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_deep_assets(
    champion: &StandardChampion,
    main: &str,
    skin: u32,
    wad_root: &Path,
    gen_bytes: &[u8],
    gen_fields: &[tree::Field],
    gen_graph: Option<u32>,
    finding: &mut SkinFinding,
) {
    let defined_keys = collect_defined_keys(champion, wad_root, gen_bytes);
    if let Some(resolver) = tree::field(gen_fields, h("mResourceResolver")).and_then(Value::as_u32)
        && resolver != 0
        && !defined_keys.contains(&resolver)
    {
        finding.stale_references.push(format!(
            "{main} skin{skin}: mResourceResolver {resolver:08x} missing from skin and linked bins"
        ));
    }
    for ui_field in [
        "iconAvatar",
        "iconCircle",
        "iconSquare",
        "loadscreen",
        "loadscreenVintage",
    ] {
        if let Some(tex) = tree::field(gen_fields, h(ui_field)).and_then(raw_text) {
            check_texture_ref(champion, main, skin, wad_root, &tex, finding);
        }
    }
    if let Some(mesh) = tree::field(gen_fields, h("skinMeshProperties")).and_then(Value::fields) {
        if let Some(skl) = tree::field(mesh, h("skeleton")).and_then(raw_text) {
            let lower = skl.to_ascii_lowercase();
            if let Some(bytes) = std::fs::read(wad_root.join(&lower))
                .ok()
                .or_else(|| champion.read_path(&lower).ok().flatten())
                && let Ok(ps) = dekan_classic::skeleton::parse(&bytes)
            {
                let len = ps.joints.len() as i16;
                if ps.joints.is_empty()
                    || ps.joints.iter().any(|j| j.parent < -1 || j.parent >= len)
                {
                    finding.stale_references.push(format!(
                        "{main} skin{skin}: invalid skeleton joint hierarchy in '{lower}'"
                    ));
                }
            }
        }
        if let Some(tex) = tree::field(mesh, h("texture")).and_then(raw_text) {
            check_texture_ref(champion, main, skin, wad_root, &tex, finding);
        }
        if let Some(items) = tree::field(mesh, h("materialOverride")).and_then(Value::items) {
            for f in items.iter().filter_map(Value::fields) {
                if let Some(tex) = tree::field(f, h("texture")).and_then(raw_text) {
                    check_texture_ref(champion, main, skin, wad_root, &tex, finding);
                }
            }
        }
    }
    check_audio_banks_and_rebase(champion, main, skin, wad_root, gen_fields, finding);
    check_vfx_materials_and_character_record(champion, main, skin, wad_root, gen_bytes, finding);
    if let Some(gk) = gen_graph
        && let Ok(parsed) = parse_prop_file(gen_bytes)
    {
        for link in &parsed.links {
            let lower = link.to_ascii_lowercase();
            if lower.contains("/animations/")
                && let Ok(bytes) = std::fs::read(wad_root.join(&lower))
            {
                check_animation_clips_and_anm(champion, main, skin, wad_root, &bytes, gk, finding);
            }
        }
    }
}
