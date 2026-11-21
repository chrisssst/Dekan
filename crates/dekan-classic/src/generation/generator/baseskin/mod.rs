use std::collections::BTreeMap;
use std::path::Path;

use dekan_wad::hash::wad_path_hash;
use dekan_wad::prop::tree::{self, Value};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};
use dekan_wad::wad::WadFile;
use tracing::{info, warn};

use crate::error::ClassicError;
use crate::gear_toggle::{bin_error, h};
use crate::vfx_markers::{raw_text, text};

const SKIN_DATA_CLASS: u32 = 0x9b67_e9f6;

struct BaseIdentity {
    skin_name: Option<String>,
    skeleton: Option<String>,
    simple_skin: Option<String>,
    tag_event_list: Option<Vec<String>>,
}

fn read_base_identity(
    wad: &WadFile,
    character: &str,
) -> Result<Option<BaseIdentity>, ClassicError> {
    let base_path = format!("data/characters/{character}/skins/skin0.bin");
    let Some(bytes) = wad.read(wad_path_hash(&base_path))? else {
        return Ok(None);
    };
    let parsed = parse_prop_file(&bytes).map_err(bin_error)?;
    let Some(entry) = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
    else {
        return Ok(None);
    };
    let fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
    let skin_name = tree::field(&fields, h("championSkinName")).and_then(raw_text);
    let mesh = tree::field(&fields, h("skinMeshProperties")).and_then(Value::fields);
    let skeleton = mesh.and_then(|m| tree::field(m, h("skeleton")).and_then(raw_text));
    let simple_skin = mesh.and_then(|m| tree::field(m, h("simpleSkin")).and_then(raw_text));
    let tag_event_list = tree::field(&fields, h("skinAudioProperties"))
        .and_then(Value::fields)
        .and_then(|a| tree::field(a, h("tagEventList")))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(raw_text).collect());
    Ok(Some(BaseIdentity {
        skin_name,
        skeleton,
        simple_skin,
        tag_event_list,
    }))
}

fn read_source_asset(
    wad: &WadFile,
    wad_root: &Path,
    asset_path: &str,
) -> Result<Option<Vec<u8>>, ClassicError> {
    let local = wad_root.join(asset_path.to_ascii_lowercase());
    if local.is_file() {
        return Ok(Some(std::fs::read(local)?));
    }
    wad.read(wad_path_hash(&asset_path.to_ascii_lowercase()))
        .map_err(ClassicError::Wad)
}

fn write_rebuilt_asset(
    wad_root: &Path,
    source_path: &str,
    base_path: &str,
    data: Vec<u8>,
) -> Result<(), ClassicError> {
    let before_bytes = data.len();
    let (data, transform) = mesh::rebuild_mesh(data);
    let after_bytes = data.len();
    let dst = wad_root.join(base_path.to_ascii_lowercase());
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dst, data)?;
    info!(
        source_path = %source_path.to_ascii_lowercase(),
        base_path = %base_path.to_ascii_lowercase(),
        transform,
        before_bytes,
        after_bytes,
        "Base-skin mesh asset rebuilt"
    );
    Ok(())
}

fn log_post_rebase_summary(character: &str, fields: &[tree::Field]) {
    let skin_name = tree::field(fields, h("championSkinName")).and_then(raw_text);
    let mesh = tree::field(fields, h("skinMeshProperties")).and_then(Value::fields);
    let skeleton = mesh.and_then(|m| tree::field(m, h("skeleton")).and_then(raw_text));
    let simple_skin = mesh.and_then(|m| tree::field(m, h("simpleSkin")).and_then(raw_text));
    let audio = tree::field(fields, h("skinAudioProperties")).and_then(Value::fields);
    let tags = audio
        .map(|a| units::string_items(a, "tagEventList"))
        .unwrap_or_default();
    let mut unit_names = Vec::new();
    let mut remaining_skin_refs = Vec::new();
    if let Some(ref name) = skin_name {
        if units::contains_skin_numbered_ref(name, character) {
            remaining_skin_refs.push(format!("championSkinName={name}"));
        }
    }
    for tag in &tags {
        if units::contains_skin_numbered_ref(tag, character) {
            remaining_skin_refs.push(format!("tagEventList={tag}"));
        }
    }
    if let Some(unit_vals) =
        audio.and_then(|a| tree::field(a, h("bankUnits")).and_then(Value::items))
    {
        for u in unit_vals {
            let Some(uf) = u.fields() else {
                continue;
            };
            if let Some(uname) = tree::field(uf, h("name")).and_then(raw_text) {
                if units::contains_skin_numbered_ref(&uname, character) {
                    remaining_skin_refs.push(format!("bankUnits.name={uname}"));
                }
                unit_names.push(uname);
            }
            for ev in units::string_items(uf, "events") {
                if units::contains_skin_numbered_ref(&ev, character) {
                    remaining_skin_refs.push(format!("bankUnits.events={ev}"));
                    break;
                }
            }
        }
    }
    info!(
        character,
        champion_skin_name = ?skin_name,
        skeleton = ?skeleton,
        simple_skin = ?simple_skin,
        tag_event_list = ?tags,
        bank_units = ?unit_names,
        remaining_skin_refs = remaining_skin_refs.len(),
        "Base-skin identity rebase summary"
    );
    if !remaining_skin_refs.is_empty() {
        warn!(
            character,
            refs = ?remaining_skin_refs,
            "Unexpected Skin<n> references remained after base-skin rebase"
        );
    }
}

pub(crate) fn rebase_slot0_identity(
    wad: &WadFile,
    character: &str,
    generated: Vec<u8>,
    wad_root: &Path,
    shipped: &mut BTreeMap<String, String>,
) -> Result<Vec<u8>, ClassicError> {
    let base = match read_base_identity(wad, character) {
        Ok(Some(info)) => info,
        Ok(None) => BaseIdentity {
            skin_name: None,
            skeleton: None,
            simple_skin: None,
            tag_event_list: None,
        },
        Err(e) => {
            warn!(character, error = %e, "Base skin0.bin unreadable; using fallback base-skin identity");
            BaseIdentity {
                skin_name: None,
                skeleton: None,
                simple_skin: None,
                tag_event_list: None,
            }
        }
    };

    let mut file = parse_prop_file(&generated).map_err(bin_error)?;
    let Some(entry) = file
        .entries
        .iter_mut()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
    else {
        return Ok(generated);
    };
    let mut fields = tree::parse_fields(&entry.body).map_err(bin_error)?;
    let mut changed = false;

    let current_skin_name = tree::field(&fields, h("championSkinName")).and_then(raw_text);
    let desired_skin_name = base.skin_name.clone().or_else(|| {
        current_skin_name
            .as_deref()
            .filter(|n| units::contains_skin_numbered_ref(n, character))
            .map(|_| character.to_owned())
    });
    if let Some(ref base_name) = desired_skin_name {
        if current_skin_name.as_deref() != Some(base_name.as_str()) {
            info!(
                character,
                skin_name_before = ?current_skin_name,
                skin_name_after = %base_name,
                "Base-skin championSkinName rebased"
            );
            tree::set_field(&mut fields, h("championSkinName"), text(base_name));
            changed = true;
        }
    }

    let targets: Vec<(&str, String)> = {
        let mesh = tree::field(&fields, h("skinMeshProperties")).and_then(Value::fields);
        let pairs = [
            ("skeleton", base.skeleton.as_deref()),
            ("simpleSkin", base.simple_skin.as_deref()),
        ];
        let mut updates = Vec::new();
        for (field_name, base_path_opt) in pairs {
            let Some(skin_path) =
                mesh.and_then(|m| tree::field(m, h(field_name)).and_then(raw_text))
            else {
                continue;
            };
            let skin_key = skin_path.to_ascii_lowercase();
            let Some(base_path) = base_path_opt else {
                if !shipped.contains_key(&skin_key)
                    && wad.contains(wad_path_hash(&skin_key))
                    && let Some(bytes) = read_source_asset(wad, wad_root, &skin_path)?
                {
                    write_rebuilt_asset(wad_root, &skin_path, &skin_path, bytes)?;
                    shipped.insert(skin_key.clone(), skin_key);
                }
                continue;
            };
            let base_key = base_path.to_ascii_lowercase();
            if let Some(prev_src) = shipped.get(&base_key) {
                if *prev_src == skin_key && skin_key != base_key {
                    updates.push((field_name, base_path.to_owned()));
                } else if *prev_src != skin_key
                    && !shipped.contains_key(&skin_key)
                    && let Some(bytes) = read_source_asset(wad, wad_root, &skin_path)?
                {
                    write_rebuilt_asset(wad_root, &skin_path, &skin_path, bytes)?;
                    shipped.insert(skin_key.clone(), skin_key);
                }
                continue;
            }
            if skin_key == base_key {
                let local = wad_root.join(&base_key);
                if local.is_file() {
                    let bytes = std::fs::read(&local)?;
                    write_rebuilt_asset(wad_root, &skin_path, base_path, bytes)?;
                }
                shipped.insert(base_key, skin_key);
                continue;
            }
            if let Some(bytes) = read_source_asset(wad, wad_root, &skin_path)? {
                write_rebuilt_asset(wad_root, &skin_path, base_path, bytes)?;
                shipped.insert(base_key, skin_key);
                updates.push((field_name, base_path.to_owned()));
            }
        }
        updates
    };

    if !targets.is_empty() {
        if let Some(props) =
            tree::field_mut(&mut fields, h("skinMeshProperties")).and_then(Value::fields_mut)
        {
            for (field_name, path) in targets {
                tree::set_field(props, h(field_name), text(&path));
                changed = true;
            }
        }
    }

    if audio::rebase_skin_audio(
        wad,
        character,
        &mut fields,
        base.tag_event_list.as_deref(),
        wad_root,
        shipped,
    )? {
        changed = true;
    }

    log_post_rebase_summary(character, &fields);

    if !changed {
        return Ok(generated);
    }
    entry.body = tree::write_fields(&fields).map_err(bin_error)?;
    serialize_prop_file(&file).map_err(bin_error)
}

mod audio;
mod banks;
mod mesh;
mod units;

#[cfg(test)]
mod tests;
