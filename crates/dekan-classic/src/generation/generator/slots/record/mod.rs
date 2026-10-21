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

pub(crate) fn object_changes(source: &[u8], generated: &[u8]) -> Vec<serde_json::Value> {
    let (Ok(before), Ok(after)) = (parse_prop_file(source), parse_prop_file(generated)) else {
        return Vec::new();
    };
    after
        .entries
        .iter()
        .map(|made| {
            let original = before
                .entries
                .iter()
                .find(|e| e.class_hash == made.class_hash);
            let changes = original.map(|o| dekan_wad::prop::diff_fields(&o.body, &made.body));
            serde_json::json!({
                "class": format!("{:08x}", made.class_hash),
                "key": format!("{:08x}", made.key_hash),
                "source_key": original.map(|o| format!("{:08x}", o.key_hash)),
                "bytes": made.body.len(),
                "field_changes": match changes {
                    Some(Ok(list)) => serde_json::to_value(list).unwrap_or_default(),
                    Some(Err(e)) => serde_json::json!({ "unreadable": e.to_string() }),
                    None => serde_json::json!({ "unreadable": "no object of this class in the source bin" }),
                },
            })
        })
        .collect()
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
    let objects = object_changes(source, generated);
    let changed_fields: usize = objects
        .iter()
        .filter_map(|o| o["field_changes"].as_array().map(Vec::len))
        .sum();
    let unreadable = objects
        .iter()
        .any(|o| o["field_changes"].get("unreadable").is_some());
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
        changed_fields,
        unreadable_objects = unreadable,
        "Skin bin generated for slot 0"
    );
    for object in &objects {
        if let Some(list) = object["field_changes"].as_array() {
            for change in list {
                debug!(
                    alias,
                    character,
                    class = %object["class"],
                    path = %change["path"],
                    before = %change["before"],
                    after = %change["after"],
                    "Generated field differs from the source bin"
                );
            }
        }
    }
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
        "links": after.links,
        "source_links": before.links,
        "classification_before": before.classification,
        "classification_after": after.classification,
        "animation_graph": graph,
        "objects": objects,
    })
}
