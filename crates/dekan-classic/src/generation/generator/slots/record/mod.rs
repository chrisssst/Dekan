use super::*;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SkinBinFacts {
    pub(crate) links: Vec<String>,
    pub(crate) classification: Option<u32>,
    animation_graph: Option<u32>,
    pub(crate) objects: usize,
}

pub(crate) fn skin_bin_facts(bytes: &[u8]) -> Option<SkinBinFacts> {
    let parsed = parse_prop_file(bytes).ok()?;
    let skin = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS);
    let read = |path: &[u32]| {
        skin.and_then(|e| field_value(&e.body, path).ok().flatten())
            .and_then(|v| v.as_u32())
    };
    Some(SkinBinFacts {
        classification: read(&[prop_key_hash(SKIN_CLASSIFICATION_FIELD)]),
        animation_graph: read(&[
            prop_key_hash("skinAnimationProperties"),
            prop_key_hash("animationGraphData"),
        ]),
        objects: parsed.entries.len(),
        links: parsed.links,
    })
}

pub(crate) fn generated_bin_record(
    alias: &str,
    character: &str,
    source_skin: u32,
    source: &[u8],
    generated: &[u8],
) -> serde_json::Value {
    let facts = |bytes: &[u8], which: &str| {
        skin_bin_facts(bytes).unwrap_or_else(|| {
            warn!(
                alias,
                character,
                source_skin,
                which,
                "Skin bin unreadable for the generation record; it lists no facts"
            );
            SkinBinFacts::default()
        })
    };
    let before = facts(source, "source");
    let after = facts(generated, "generated");
    let source_checksum = format!("{:016x}", dekan_wad::hash::content_checksum(source));
    let generated_checksum = format!("{:016x}", dekan_wad::hash::content_checksum(generated));
    let graph = after.animation_graph.map(|h| format!("{h:08x}"));
    info!(
        alias,
        character,
        source_skin,
        source_bytes = source.len(),
        source_checksum = %source_checksum,
        generated_bytes = generated.len(),
        generated_checksum = %generated_checksum,
        source_objects = before.objects,
        kept_objects = after.objects,
        links = after.links.len(),
        classification_before = ?before.classification,
        classification_after = ?after.classification,
        animation_graph = ?graph,
        animation_graph_is_source = before.animation_graph == after.animation_graph,
        "Skin bin generated for slot 0"
    );
    debug!(alias, character, source_skin, links = ?after.links, "Skin bin links");
    serde_json::json!({
        "character": character,
        "source_skin": source_skin,
        "file": format!("data/characters/{character}/skins/skin0.bin"),
        "source_file": skin_bin(character, source_skin),
        "source_bytes": source.len(),
        "source_checksum": source_checksum,
        "generated_bytes": generated.len(),
        "generated_checksum": generated_checksum,
        "source_objects": before.objects,
        "kept_objects": after.objects,
        "links": after.links,
        "classification_before": before.classification,
        "classification_after": after.classification,
        "animation_graph": graph,
    })
}
