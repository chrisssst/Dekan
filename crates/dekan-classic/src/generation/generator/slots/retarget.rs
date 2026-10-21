use super::*;

pub fn retarget_skin_bin(
    source: &[u8],
    character: &str,
    source_skin: u32,
    target_skin: u32,
    identity: Option<SlotIdentity>,
) -> Result<Vec<u8>, ClassicError> {
    let source_prefix = format!("Characters/{character}/Skins/Skin{source_skin}");
    let target_prefix = format!("Characters/{character}/Skins/Skin{target_skin}");
    let mut skin_source_hash = prop_key_hash(&source_prefix);
    let skin_target_hash = prop_key_hash(&target_prefix);

    let resources_source = format!("{source_prefix}/Resources");
    let resources_target = format!("{target_prefix}/Resources");
    let mut resources_source_hash = prop_key_hash(&resources_source);
    let resources_target_hash = prop_key_hash(&resources_target);

    let parsed = parse_prop_file(source).map_err(|e| ClassicError::Bin(e.to_string()))?;

    if !parsed
        .entries
        .iter()
        .any(|e| e.key_hash == skin_source_hash)
    {
        let alt_prefix = format!(
            "Characters/{}/Skins/Skin{source_skin}",
            character.to_ascii_lowercase()
        );
        let alt_hash = prop_key_hash(&alt_prefix);
        if parsed.entries.iter().any(|e| e.key_hash == alt_hash) {
            skin_source_hash = alt_hash;
            let alt_resources = format!("{alt_prefix}/Resources");
            resources_source_hash = prop_key_hash(&alt_resources);
        }
    }

    let mut selected: Vec<PropEntry> = parsed
        .entries
        .into_iter()
        .filter_map(|entry| {
            let renamed = if entry.key_hash == skin_source_hash {
                skin_target_hash
            } else if entry.key_hash == resources_source_hash {
                resources_target_hash
            } else {
                return None;
            };
            Some(PropEntry {
                key_hash: renamed,
                ..entry
            })
        })
        .collect();

    let moved = std::collections::BTreeMap::from([
        (skin_source_hash, skin_target_hash),
        (resources_source_hash, resources_target_hash),
    ]);
    for entry in &mut selected {
        remap_references(&mut entry.body, &moved).map_err(|e| {
            ClassicError::Bin(format!(
                "references in {source_prefix} could not be walked: {e}"
            ))
        })?;
    }

    if let Some(identity) = identity {
        for entry in selected
            .iter_mut()
            .filter(|e| e.key_hash == skin_target_hash)
        {
            let fields = identity
                .classification
                .map(|value| (SKIN_CLASSIFICATION_FIELD, value))
                .into_iter()
                .chain(std::iter::once((SKIN_PARENT_FIELD, identity.parent)));
            for (name, value) in fields {
                set_int_field(&mut entry.body, prop_key_hash(name), value).map_err(|e| {
                    ClassicError::Bin(format!("{name} in {source_prefix} could not be set: {e}"))
                })?;
            }
        }
    }

    if !selected.iter().any(|e| e.key_hash == skin_target_hash) {
        return Err(ClassicError::Bin(format!(
            "{source_prefix} object not found in the skin bin"
        )));
    }

    let mut links = vec![format!(
        "DATA/Characters/{character}/Skins/Skin{source_skin}.bin"
    )];
    for link in parsed.links {
        if !links.contains(&link) {
            links.push(link);
        }
    }

    serialize_prop_file(&PropFile {
        version: parsed.version,
        links,
        entries: selected,
    })
    .map_err(|e| ClassicError::Bin(e.to_string()))
}

pub fn relocate_prop(
    bytes: &[u8],
    moves: &std::collections::BTreeMap<u32, u32>,
    extra_link: Option<&str>,
) -> Result<Vec<u8>, ClassicError> {
    let mut parsed = parse_prop_file(bytes).map_err(|e| ClassicError::Bin(e.to_string()))?;
    for entry in &mut parsed.entries {
        if let Some(&target) = moves.get(&entry.key_hash) {
            entry.key_hash = target;
        }
        remap_references(&mut entry.body, moves)
            .map_err(|e| ClassicError::Bin(format!("references could not be walked: {e}")))?;
    }
    if let Some(link) = extra_link {
        if !parsed.links.iter().any(|l| l.eq_ignore_ascii_case(link)) {
            parsed.links.push(link.to_owned());
        }
    }
    serialize_prop_file(&parsed).map_err(|e| ClassicError::Bin(e.to_string()))
}

pub(crate) struct FormCycle {
    pub(crate) files: Vec<(String, Vec<u8>)>,
    pub(crate) skin0: Vec<u8>,
    pub(crate) forms: usize,
    pub(crate) drivers: usize,
    pub(crate) effects: usize,
    pub(crate) coverage: crate::form_gear::GearCoverage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenerationOptions {
    pub graph_in_slot0: bool,
    pub chroma_keeps_classification: bool,
}

pub fn move_graph_to_slot0(
    wad: &WadFile,
    character: &str,
    source_skin_bin: &[u8],
    generated: Vec<u8>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), ClassicError> {
    let parsed = parse_prop_file(source_skin_bin).map_err(|e| ClassicError::Bin(e.to_string()))?;
    let Some(skin) = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
    else {
        return Ok((generated, None));
    };
    let graph = field_value(
        &skin.body,
        &[
            prop_key_hash("skinAnimationProperties"),
            prop_key_hash("animationGraphData"),
        ],
    )
    .map_err(|e| ClassicError::Bin(e.to_string()))?
    .and_then(|v| v.as_u32());
    let Some(graph) = graph else {
        return Ok((generated, None));
    };
    let slot0_graph = prop_key_hash(&format!("Characters/{character}/Animations/Skin0"));
    if graph == slot0_graph {
        return Ok((generated, None));
    }
    for link in parsed
        .links
        .iter()
        .filter(|l| l.to_ascii_lowercase().contains("/animations/"))
    {
        let Some(bytes) = wad.read(wad_path_hash(&link.to_ascii_lowercase()))? else {
            continue;
        };
        let holds_graph = match parse_prop_file(&bytes) {
            Ok(anim) => anim.entries.iter().any(|e| e.key_hash == graph),
            Err(e) => {
                warn!(character, link = %link, error = %e, "Linked animation bin unreadable while looking for the skin's graph");
                false
            }
        };
        if !holds_graph {
            continue;
        }
        let moves = std::collections::BTreeMap::from([(graph, slot0_graph)]);
        let anim = relocate_prop(&bytes, &moves, Some(link))?;
        let slot0_link = format!("DATA/Characters/{character}/Animations/Skin0.bin");
        let skin_bin = relocate_prop(&generated, &moves, Some(&slot0_link))?;
        return Ok((skin_bin, Some(anim)));
    }
    warn!(
        character,
        graph = format!("{graph:08x}"),
        "The skin's animation graph is not in any animation bin it links; it stays where the game has it"
    );
    Ok((generated, None))
}

pub fn retarget_animation_bin(
    source: &[u8],
    character: &str,
    source_skin: u32,
    target_skin: u32,
) -> Result<Vec<u8>, ClassicError> {
    let source_prefix = format!("Characters/{character}/Animations/Skin{source_skin}");
    let target_prefix = format!("Characters/{character}/Animations/Skin{target_skin}");
    let mut anim_source_hash = prop_key_hash(&source_prefix);
    let anim_target_hash = prop_key_hash(&target_prefix);

    let parsed = parse_prop_file(source).map_err(|e| ClassicError::Bin(e.to_string()))?;

    if !parsed
        .entries
        .iter()
        .any(|e| e.key_hash == anim_source_hash)
    {
        let alt_prefix = format!(
            "Characters/{}/Animations/Skin{source_skin}",
            character.to_ascii_lowercase()
        );
        let alt_hash = prop_key_hash(&alt_prefix);
        if parsed.entries.iter().any(|e| e.key_hash == alt_hash) {
            anim_source_hash = alt_hash;
        }
    }

    let selected: Vec<PropEntry> = parsed
        .entries
        .into_iter()
        .map(|entry| {
            let key_hash = if entry.key_hash == anim_source_hash {
                anim_target_hash
            } else {
                entry.key_hash
            };
            PropEntry { key_hash, ..entry }
        })
        .collect();

    if !selected.iter().any(|e| e.key_hash == anim_target_hash) {
        return Err(ClassicError::Bin(format!(
            "{source_prefix} object not found in the animation bin"
        )));
    }

    let mut links = parsed.links;
    let source_link = format!("DATA/Characters/{character}/Animations/Skin{source_skin}.bin");
    if !links.contains(&source_link) {
        links.push(source_link);
    }

    serialize_prop_file(&PropFile {
        version: parsed.version,
        links,
        entries: selected,
    })
    .map_err(|e| ClassicError::Bin(e.to_string()))
}
