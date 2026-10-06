use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use dekan_wad::hash::{prop_key_hash, wad_path_hash};
use dekan_wad::prop::{
    PropEntry, PropFile, field_value, parse_prop_file, remap_references, serialize_prop_file,
    set_int_field,
};
use dekan_wad::wad::WadFile;
use tracing::{debug, info, warn};

use crate::builder::CLASSIC_DEFAULT_SLOTS;
use crate::error::ClassicError;

pub const CLASSIC_MOD_PREFIX: &str = "classic_";

const SKIN_CLASSIFICATION_FIELD: &str = "skinClassification";
const SKIN_PARENT_FIELD: &str = "skinParent";

#[must_use]
pub fn is_safe_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[must_use]
pub fn skin_number(skin_or_chroma_id: u32) -> u32 {
    crate::builder::ClassicIdMapper::normalize_skin_id(skin_or_chroma_id) % 1000
}

#[must_use]
pub fn main_character(alias: &str) -> String {
    format!("jade_{}", alias.to_ascii_lowercase())
}

fn character_bin(character: &str) -> String {
    format!("data/characters/{character}/{character}.bin")
}

fn skin_bin(character: &str, skin: u32) -> String {
    format!("data/characters/{character}/skins/skin{skin}.bin")
}

fn animation_bin(character: &str, skin: u32) -> String {
    format!("data/characters/{character}/animations/skin{skin}.bin")
}

const SKIN_DATA_CLASS: u32 = 0x9b67_e9f6;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlotIdentity {
    pub classification: Option<u32>,
    pub parent: u32,
}

pub fn slot_identity(slot_bin: &[u8]) -> Result<SlotIdentity, ClassicError> {
    let parsed = parse_prop_file(slot_bin).map_err(|e| ClassicError::Bin(e.to_string()))?;
    let skin = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
        .ok_or_else(|| ClassicError::Bin("no skin object in the slot bin".into()))?;
    let read = |name: &str| {
        field_value(&skin.body, &[prop_key_hash(name)])
            .map(|v| v.and_then(|v| v.as_u32()))
            .map_err(|e| ClassicError::Bin(e.to_string()))
    };
    Ok(SlotIdentity {
        classification: read(SKIN_CLASSIFICATION_FIELD)?,
        parent: read(SKIN_PARENT_FIELD)?.unwrap_or(0),
    })
}

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

#[derive(Debug, Default, PartialEq, Eq)]
struct SkinBinFacts {
    links: Vec<String>,
    classification: Option<u32>,
    animation_graph: Option<u32>,
    objects: usize,
}

fn skin_bin_facts(bytes: &[u8]) -> Option<SkinBinFacts> {
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

fn object_changes(source: &[u8], generated: &[u8]) -> Vec<serde_json::Value> {
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

fn generated_bin_record(
    alias: &str,
    character: &str,
    source_skin: u32,
    source: &[u8],
    generated: &[u8],
) -> serde_json::Value {
    let before = skin_bin_facts(source).unwrap_or_default();
    let after = skin_bin_facts(generated).unwrap_or_default();
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

struct FormCycle {
    files: Vec<(String, Vec<u8>)>,
    skin0: Vec<u8>,
    forms: usize,
    drivers: usize,
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
        let holds_graph = parse_prop_file(&bytes)
            .map(|anim| anim.entries.iter().any(|e| e.key_hash == graph))
            .unwrap_or(false);
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

#[must_use]
pub fn jade_characters(hashes_path: &Path, cache_path: &Path) -> BTreeSet<String> {
    let meta = match std::fs::metadata(hashes_path) {
        Ok(meta) => meta,
        Err(e) => {
            warn!(
                table = %hashes_path.display(),
                error = %e,
                "Game hash table unavailable; Rift Classic companion characters will be recovered from the champion bins"
            );
            return BTreeSet::new();
        }
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let fingerprint = format!("{}:{mtime}", meta.len());

    if let Ok(bytes) = std::fs::read(cache_path) {
        if let Ok(cached) = serde_json::from_slice::<CharacterCache>(&bytes) {
            if cached.source == fingerprint {
                debug!(
                    characters = cached.characters.len(),
                    "Rift Classic character index from cache"
                );
                return cached.characters;
            }
        }
    }

    let characters = match scan_jade_characters(hashes_path) {
        Ok(characters) => characters,
        Err(e) => {
            warn!(
                table = %hashes_path.display(),
                error = %e,
                "Game hash table could not be read; Rift Classic companion characters will be recovered from the champion bins"
            );
            return BTreeSet::new();
        }
    };

    let cache = CharacterCache {
        source: fingerprint,
        characters: characters.clone(),
    };
    match serde_json::to_vec(&cache) {
        Ok(bytes) => {
            if let Some(parent) = cache_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    debug!(error = %e, "Rift Classic character cache folder unavailable");
                }
            }
            if let Err(e) = std::fs::write(cache_path, bytes) {
                debug!(error = %e, "Rift Classic character index could not be cached");
            }
        }
        Err(e) => debug!(error = %e, "Rift Classic character index could not be serialized"),
    }
    info!(
        characters = characters.len(),
        "Rift Classic characters indexed from the game hash table"
    );
    characters
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CharacterCache {
    source: String,
    characters: BTreeSet<String>,
}

fn scan_jade_characters(hashes_path: &Path) -> std::io::Result<BTreeSet<String>> {
    use std::io::BufRead;

    const NEEDLE: &[u8] = b"data/characters/jade_";
    let file = std::fs::File::open(hashes_path)?;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    let mut line = Vec::new();
    let mut found = BTreeSet::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let Some(start) = line.windows(NEEDLE.len()).position(|w| w == NEEDLE) else {
            continue;
        };
        let name_start = start + b"data/characters/".len();
        let name: Vec<u8> = line[name_start..]
            .iter()
            .copied()
            .take_while(|b| *b != b'/')
            .collect();

        let terminated = line.get(name_start + name.len()) == Some(&b'/');
        if terminated
            && name
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
        {
            if let Ok(name) = String::from_utf8(name) {
                found.insert(name);
            }
        }
    }
    Ok(found)
}

fn wad_stamp(path: &Path) -> String {
    std::fs::metadata(path)
        .map(|meta| {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!("{}:{mtime}", meta.len())
        })
        .unwrap_or_default()
}

fn character_names_in_bins(wad: &WadFile, alias: &str) -> BTreeSet<String> {
    const MAX_BIN_BYTES: usize = 8 * 1024 * 1024;
    const NEEDLE: &[u8] = b"characters/";

    let mut found = BTreeSet::new();
    let mut unreadable = 0usize;
    for (hash, size) in wad.entries() {
        if size > MAX_BIN_BYTES {
            continue;
        }
        match wad.read_prefix(hash, 4) {
            Ok(Some(head)) if head.starts_with(b"PROP") || head.starts_with(b"PTCH") => {}
            Ok(_) => continue,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        }
        let bytes = match wad.read(hash) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        };
        if !(bytes.starts_with(b"PROP") || bytes.starts_with(b"PTCH")) {
            continue;
        }
        let lower = bytes.to_ascii_lowercase();
        let mut from = 0;
        while let Some(pos) = lower[from..]
            .windows(NEEDLE.len())
            .position(|w| w == NEEDLE)
        {
            let name_start = from + pos + NEEDLE.len();
            let name: Vec<u8> = lower[name_start..]
                .iter()
                .copied()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .collect();
            if !name.is_empty() && lower.get(name_start + name.len()) == Some(&b'/') {
                if let Ok(name) = String::from_utf8(name) {
                    found.insert(name);
                }
            }
            from = name_start;
        }
    }
    if unreadable > 0 {
        warn!(
            alias,
            unreadable,
            "Some entries of the champion WAD could not be read while looking for character names"
        );
    }
    found
}

fn cached_names(
    cache_path: &Path,
    stamp: &str,
    alias: &str,
    ahead_of_time: bool,
    scan: impl FnOnce() -> BTreeSet<String>,
) -> BTreeSet<String> {
    if !stamp.is_empty() {
        if let Ok(bytes) = std::fs::read(cache_path) {
            if let Ok(cached) = serde_json::from_slice::<CharacterCache>(&bytes) {
                if cached.source == stamp {
                    debug!(
                        alias,
                        characters = cached.characters.len(),
                        "Character names from the bin-scan cache"
                    );
                    return cached.characters;
                }
            }
        }
    }

    let started = std::time::Instant::now();
    let characters = scan();
    if ahead_of_time {
        debug!(
            alias,
            names = ?characters,
            elapsed_ms = started.elapsed().as_millis(),
            "Character names indexed ahead of champion select"
        );
    } else {
        info!(
            alias,
            names = ?characters,
            elapsed_ms = started.elapsed().as_millis(),
            "Character names recovered from the champion's bins"
        );
    }
    if !stamp.is_empty() {
        let cache = CharacterCache {
            source: stamp.to_owned(),
            characters: characters.clone(),
        };
        match serde_json::to_vec(&cache) {
            Ok(bytes) => {
                if let Some(parent) = cache_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        debug!(error = %e, "Bin-scan cache folder unavailable");
                    }
                }
                if let Err(e) = std::fs::write(cache_path, bytes) {
                    debug!(error = %e, "Bin-scan cache not written; the next build scans again");
                }
            }
            Err(e) => debug!(error = %e, "Bin-scan cache not serialized"),
        }
    }
    characters
}

pub struct ClassicChampion {
    alias: String,
    wad: WadFile,

    wad_stamp: String,
    main: String,
    main_display: String,
}

impl ClassicChampion {
    pub fn open(game_dir: &Path, alias: &str) -> Result<Self, ClassicError> {
        if !is_safe_alias(alias) {
            return Err(ClassicError::InvalidAlias(alias.to_owned()));
        }
        let path = game_dir
            .join("DATA")
            .join("FINAL")
            .join("Champions")
            .join(format!("{alias}.wad.client"));
        if !path.is_file() {
            return Err(ClassicError::ChampionNotFound {
                alias: format!("{alias} (no {})", path.display()),
            });
        }
        Ok(Self {
            alias: alias.to_owned(),
            wad: WadFile::open(&path)?,
            wad_stamp: wad_stamp(&path),
            main: main_character(alias),
            main_display: format!("Jade_{alias}"),
        })
    }

    #[must_use]
    pub fn with_client_character(mut self, classic_alias: Option<&str>) -> Self {
        let Some(display) = classic_alias.filter(|name| is_safe_alias(name)) else {
            return self;
        };
        let name = display.to_ascii_lowercase();
        if name == self.main {
            return self;
        }
        if self.wad.contains(wad_path_hash(&character_bin(&name))) {
            info!(
                alias = %self.alias,
                derived = %self.main,
                client = %name,
                "Rift Classic character named by the client"
            );
            self.main = name;
            self.main_display = display.to_owned();
        } else {
            warn!(
                alias = %self.alias,
                client = %name,
                derived = %self.main,
                "The client's Rift Classic character is not in the champion archive; keeping the derived name"
            );
        }
        self
    }

    #[must_use]
    pub fn main_character(&self) -> &str {
        &self.main
    }

    #[must_use]
    pub fn jade_names_in_bins(&self) -> BTreeSet<String> {
        character_names_in_bins(&self.wad, &self.alias)
            .into_iter()
            .filter(|name| name.starts_with("jade_"))
            .collect()
    }

    #[must_use]
    pub fn jade_names_from_bins_cached(&self, cache_dir: &Path) -> BTreeSet<String> {
        let cache_path = cache_dir.join(format!(
            "classic_bin_names_{}.json",
            self.alias.to_ascii_lowercase()
        ));
        cached_names(&cache_path, &self.wad_stamp, &self.alias, false, || {
            self.jade_names_in_bins()
        })
    }

    #[must_use]
    pub fn present_characters(&self, known: &BTreeSet<String>) -> Vec<String> {
        let mut candidates: BTreeSet<String> = known.clone();
        candidates.insert(self.main.clone());
        candidates
            .into_iter()
            .filter(|c| self.wad.contains(wad_path_hash(&character_bin(c))))
            .collect()
    }

    #[must_use]
    pub fn has_skin(&self, character: &str, skin: u32) -> bool {
        self.wad.contains(wad_path_hash(&skin_bin(character, skin)))
    }

    #[must_use]
    pub fn has_animation(&self, character: &str, skin: u32) -> bool {
        self.wad
            .contains(wad_path_hash(&animation_bin(character, skin)))
    }

    #[must_use]
    pub fn skin_numbers(&self, character: &str, limit: u32) -> Vec<u32> {
        (0..limit)
            .filter(|n| self.has_skin(character, *n))
            .collect()
    }

    pub fn build_mod(
        &self,
        skin: u32,
        slots: &[u32],
        known_characters: &BTreeSet<String>,
        mods_dir: &Path,
    ) -> Result<String, ClassicError> {
        let main = self.main.clone();
        let present = self.present_characters(known_characters);
        if !present.contains(&main) {
            return Err(ClassicError::ChampionNotFound {
                alias: format!("{} has no Rift Classic version in this patch", self.alias),
            });
        }
        let targets: Vec<&String> = present.iter().filter(|c| self.has_skin(c, skin)).collect();
        if targets.is_empty() {
            return Err(ClassicError::SkinNotFound {
                champion_id: 0,
                skin_id: skin,
            });
        }

        let folder = format!(
            "{CLASSIC_MOD_PREFIX}{}_{skin}",
            self.alias.to_ascii_lowercase()
        );
        let final_dir = mods_dir.join(&folder);
        let partial = mods_dir.join(format!("{folder}.partial"));
        remove_if_present(&partial)?;

        let mut written = 0usize;
        for character in &targets {
            let display = if **character == main {
                self.main_display.clone()
            } else {
                (*character).clone()
            };
            let source = self
                .wad
                .read(wad_path_hash(&skin_bin(character, skin)))?
                .ok_or_else(|| ClassicError::Bin(format!("{character} skin{skin}.bin vanished")))?;

            let bins_dir = partial
                .join("WAD")
                .join(format!("{}.wad.client", self.alias))
                .join("data")
                .join("characters")
                .join(character.as_str())
                .join("skins");
            std::fs::create_dir_all(&bins_dir)?;
            for slot in slots.iter().copied().filter(|slot| *slot != skin) {
                let identity = identity_at(&self.wad, character, slot);
                let bin = retarget_skin_bin(&source, &display, skin, slot, identity)?;
                std::fs::write(bins_dir.join(format!("skin{slot}.bin")), bin)?;
                written += 1;
            }

            let anim_target = animation_bin(character, skin);
            if self.wad.contains(wad_path_hash(&anim_target)) {
                if let Ok(Some(anim_source)) = self.wad.read(wad_path_hash(&anim_target)) {
                    let anim_dir = partial
                        .join("WAD")
                        .join(format!("{}.wad.client", self.alias))
                        .join("data")
                        .join("characters")
                        .join(character.as_str())
                        .join("animations");
                    let _ = std::fs::create_dir_all(&anim_dir); // ignore-ok: classic anim dir
                    for slot in slots.iter().copied().filter(|slot| *slot != skin) {
                        if let Ok(retargeted_anim) =
                            retarget_animation_bin(&anim_source, &display, skin, slot)
                        {
                            // ignore-ok: classic anim slot write
                            let _ = std::fs::write(
                                anim_dir.join(format!("skin{slot}.bin")),
                                retargeted_anim,
                            );
                        }
                    }
                }
            }
        }

        let meta = partial.join("META");
        std::fs::create_dir_all(&meta)?;
        let info = serde_json::json!({
            "Author": "Dekan",
            "Name": format!("{} skin {skin} (Rift Classic)", self.alias),
            "Version": "1.0",
            "Description": "Generated from installed game data",
        });
        std::fs::write(meta.join("info.json"), info.to_string())?;

        remove_if_present(&final_dir)?;
        std::fs::rename(&partial, &final_dir)?;

        info!(
            alias = %self.alias,
            skin,
            characters = ?targets,
            slots = ?slots,
            bins = written,
            folder = %folder,
            "Rift Classic mod generated from the installed game"
        );
        Ok(folder)
    }
}

fn identity_at(wad: &WadFile, character: &str, slot: u32) -> Option<SlotIdentity> {
    let path = skin_bin(character, slot);
    match wad.read(wad_path_hash(&path)) {
        Ok(Some(bin)) => match slot_identity(&bin) {
            Ok(identity) => Some(identity),
            Err(e) => {
                warn!(character, slot, error = %e, "Slot identity unreadable; the source skin keeps its own");
                None
            }
        },
        Ok(None) => None,
        Err(e) => {
            warn!(character, slot, error = %e, "Slot bin unreadable; the source skin keeps its own identity");
            None
        }
    }
}

fn remove_if_present(dir: &Path) -> Result<(), ClassicError> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[must_use]
pub fn slots_for(client_skin_id: Option<u32>) -> Vec<u32> {
    let mut slots = CLASSIC_DEFAULT_SLOTS.to_vec();
    if let Some(current) = client_skin_id.map(skin_number) {
        if !slots.contains(&current) {
            slots.push(current);
        }
    }
    slots
}

pub const STANDARD_MOD_PREFIX: &str = "std_";

#[must_use]
pub fn is_generated_folder(name: &str) -> bool {
    name.starts_with(CLASSIC_MOD_PREFIX) || name.starts_with(STANDARD_MOD_PREFIX)
}

type SharedScan = std::sync::Arc<std::sync::OnceLock<BTreeSet<String>>>;

fn shared_scan(alias: &str, stamp: &str) -> SharedScan {
    static SCANS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, SharedScan>>,
    > = std::sync::OnceLock::new();
    if stamp.is_empty() {
        return SharedScan::default();
    }
    let prefix = format!("{}|", alias.to_ascii_lowercase());
    let key = format!("{prefix}{stamp}");
    let scans = SCANS.get_or_init(Default::default);
    let mut map = match scans.lock() {
        Ok(map) => map,
        Err(poisoned) => poisoned.into_inner(),
    };
    map.retain(|k, _| !k.starts_with(&prefix) || *k == key);
    std::sync::Arc::clone(map.entry(key).or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrewarmGate {
    Go,
    Wait,
    Stop,
}

const PREWARM_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

#[must_use]
pub fn champion_aliases(game_dir: &Path) -> Vec<String> {
    let dir = game_dir.join("DATA").join("FINAL").join("Champions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut aliases: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let alias = name.strip_suffix(".wad.client")?;
            (!alias.contains('.') && is_safe_alias(alias)).then(|| alias.to_owned())
        })
        .collect();
    aliases.sort_unstable();
    aliases
}

pub fn prewarm_companions<G>(game_dir: &Path, cache_dir: &Path, gate: G)
where
    G: Fn() -> PrewarmGate + Send + 'static,
{
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RUNNING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    let game_dir = game_dir.to_path_buf();
    let cache_dir = cache_dir.to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("dekan-companion-prewarm".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let aliases = champion_aliases(&game_dir);
            let mut indexed = 0usize;
            'champions: for alias in &aliases {
                loop {
                    match gate() {
                        PrewarmGate::Go => break,
                        PrewarmGate::Wait => std::thread::sleep(PREWARM_WAIT),
                        PrewarmGate::Stop => break 'champions,
                    }
                }
                match StandardChampion::open(&game_dir, alias) {
                    Ok(champion) => {
                        champion.with_cache_dir(&cache_dir).scanned_names(true);
                        indexed += 1;
                    }
                    Err(e) => debug!(alias, error = %e, "Champion not indexed ahead of time"),
                }
            }
            info!(
                champions = aliases.len(),
                indexed,
                elapsed_s = started.elapsed().as_secs(),
                "Companion characters indexed ahead of champion select"
            );
            RUNNING.store(false, std::sync::atomic::Ordering::Release);
        });
    if let Err(e) = spawned {
        RUNNING.store(false, std::sync::atomic::Ordering::Release);
        warn!(error = %e, "Companion prewarm not started; each champion is indexed when picked");
    }
}

#[derive(Debug)]
pub struct StandardChampion {
    pub alias: String,
    wad: WadFile,
    wad_stamp: String,
    cache_dir: Option<PathBuf>,
    scanned: SharedScan,
    options: GenerationOptions,
}

impl StandardChampion {
    pub fn open(game_dir: &Path, alias: &str) -> Result<Self, ClassicError> {
        if !is_safe_alias(alias) {
            return Err(ClassicError::InvalidAlias(alias.to_owned()));
        }
        let wad_path = game_dir
            .join("DATA")
            .join("FINAL")
            .join("Champions")
            .join(format!("{alias}.wad.client"));
        let wad = WadFile::open(&wad_path).map_err(ClassicError::Wad)?;
        let stamp = wad_stamp(&wad_path);
        Ok(Self {
            alias: alias.to_owned(),
            wad,
            scanned: shared_scan(alias, &stamp),
            wad_stamp: stamp,
            cache_dir: None,
            options: GenerationOptions::default(),
        })
    }

    #[must_use]
    pub fn with_options(mut self, options: GenerationOptions) -> Self {
        self.options = options;
        self
    }

    fn slot0_identity(&self, character: &str) -> Option<SlotIdentity> {
        identity_at(&self.wad, character, 0).map(|identity| SlotIdentity {
            classification: identity
                .classification
                .filter(|_| !self.options.chroma_keeps_classification),
            ..identity
        })
    }

    fn finish_slot0(
        &self,
        character: &str,
        display: &str,
        source: &[u8],
        generated: Vec<u8>,
        characters_dir: &Path,
    ) -> Result<Vec<u8>, ClassicError> {
        if !self.options.graph_in_slot0 {
            return Ok(generated);
        }
        let (skin_bin, graph) = move_graph_to_slot0(&self.wad, display, source, generated)?;
        if let Some(graph) = graph {
            let dir = characters_dir.join(character).join("animations");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("skin0.bin"), graph)?;
            info!(character, "Animation graph moved to slot 0 (test variant)");
        }
        Ok(skin_bin)
    }

    #[must_use]
    pub fn with_cache_dir(mut self, cache_dir: &Path) -> Self {
        self.cache_dir = Some(cache_dir.to_path_buf());
        self
    }

    #[must_use]
    pub fn has_skin(&self, skin: u32) -> bool {
        let main = self.alias.to_ascii_lowercase();
        self.wad.contains(wad_path_hash(&skin_bin(&main, skin)))
    }

    #[must_use]
    pub fn skin_numbers(&self, limit: u32) -> Vec<u32> {
        (0..limit).filter(|n| self.has_skin(*n)).collect()
    }

    fn scanned_names(&self, ahead_of_time: bool) -> &BTreeSet<String> {
        let main = self.alias.to_ascii_lowercase();
        self.scanned.get_or_init(|| match &self.cache_dir {
            Some(dir) => cached_names(
                &dir.join(format!("companion_names_{main}.json")),
                &self.wad_stamp,
                &self.alias,
                ahead_of_time,
                || character_names_in_bins(&self.wad, &self.alias),
            ),
            None => character_names_in_bins(&self.wad, &self.alias),
        })
    }

    #[must_use]
    pub fn companions(&self) -> BTreeSet<String> {
        let main = self.alias.to_ascii_lowercase();
        let mut names: BTreeSet<String> = self.scanned_names(false).clone();
        names.remove(&main);
        names.retain(|name| is_safe_alias(name) && !name.starts_with("jade_"));
        names
    }

    #[must_use]
    pub fn companion_source_skin(
        &self,
        companion: &str,
        skin: u32,
        base_skin: Option<u32>,
    ) -> Option<u32> {
        std::iter::once(skin)
            .chain(base_skin.filter(|base| *base != skin && *base != 0))
            .find(|n| self.wad.contains(wad_path_hash(&skin_bin(companion, *n))))
    }

    #[must_use]
    pub fn parent_skin(&self, skin: u32) -> Option<u32> {
        let main = self.alias.to_ascii_lowercase();
        let bin = self.read_skin_bin(&main, skin).ok().flatten()?;
        let parent = slot_identity(&bin).ok()?.parent;
        (parent != 0 && parent != skin).then_some(parent)
    }

    #[must_use]
    pub fn contains_path(&self, path: &str) -> bool {
        self.wad.contains(wad_path_hash(&path.to_ascii_lowercase()))
    }

    pub fn read_skin_bin(
        &self,
        character: &str,
        skin: u32,
    ) -> Result<Option<Vec<u8>>, ClassicError> {
        Ok(self.wad.read(wad_path_hash(&skin_bin(character, skin)))?)
    }

    pub fn gear_count(&self, skin: u32) -> usize {
        let main = self.alias.to_ascii_lowercase();
        self.read_skin_bin(&main, skin)
            .ok()
            .flatten()
            .and_then(|bin| crate::forms::gear_keys(&bin).ok())
            .map_or(0, |keys| keys.len())
    }

    pub fn build_mod(
        &self,
        skin: u32,
        base_skin: Option<u32>,
        mods_dir: &Path,
    ) -> Result<String, ClassicError> {
        self.build(skin, base_skin, None, mods_dir)
    }

    pub fn build_mod_form(
        &self,
        skin: u32,
        base_skin: Option<u32>,
        form: u32,
        mods_dir: &Path,
    ) -> Result<String, ClassicError> {
        self.build(skin, base_skin, Some(form), mods_dir)
    }

    fn bake_form(
        &self,
        source: &[u8],
        generated: Vec<u8>,
        form: u32,
    ) -> Result<Vec<u8>, ClassicError> {
        let keys = crate::forms::gear_keys(source)?;
        let key = *keys.get(form as usize).ok_or_else(|| {
            ClassicError::Bin(format!(
                "form {form} does not exist; the skin has {} forms",
                keys.len()
            ))
        })?;
        let gear = self.gear_body(source, key)?;
        let mut file = parse_prop_file(&generated).map_err(|e| ClassicError::Bin(e.to_string()))?;
        let submeshes = self.submeshes_of(&file, &gear)?;
        crate::forms::bake_form(
            &mut file,
            &crate::forms::GearForm {
                index: form,
                gear_body: &gear,
                submeshes: &submeshes,
            },
        )?;
        serialize_prop_file(&file).map_err(|e| ClassicError::Bin(e.to_string()))
    }

    fn gear_body(&self, source: &[u8], key: u32) -> Result<Vec<u8>, ClassicError> {
        let parsed = parse_prop_file(source).map_err(|e| ClassicError::Bin(e.to_string()))?;
        if let Some(entry) = parsed.entries.iter().find(|e| e.key_hash == key) {
            return Ok(entry.body.clone());
        }
        crate::forms::find_linked_object(&self.wad, &parsed.links, key)?.ok_or_else(|| {
            ClassicError::Bin(format!(
                "gear {key:08x} is neither in the skin's bin nor in the bins it links"
            ))
        })
    }

    fn form_cycle(
        &self,
        source: &[u8],
        source_path: &str,
        generated: &[u8],
    ) -> Result<Option<FormCycle>, ClassicError> {
        let keys = crate::forms::gear_keys(source)?;
        if keys.len() < 2 || self.gear_count(0) > 0 {
            return Ok(None);
        }
        let swaps = keys
            .iter()
            .map(|key| {
                self.gear_body(source, *key)
                    .and_then(|body| crate::gear_toggle::gear_swap(&body))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let Some(markers) = crate::gear_toggle::markers(&swaps) else {
            return Ok(None);
        };
        let parsed = parse_prop_file(source).map_err(|e| ClassicError::Bin(e.to_string()))?;
        let Some(skin) = parsed
            .entries
            .iter()
            .find(|e| e.class_hash == SKIN_DATA_CLASS)
        else {
            return Ok(None);
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
            return Ok(None);
        };
        if graph == prop_key_hash(&format!("Characters/{}/Animations/Skin0", self.alias)) {
            return Ok(None);
        }
        let mut toggled = None;
        for link in parsed
            .links
            .iter()
            .filter(|l| l.to_ascii_lowercase().contains("/animations/"))
        {
            let path = link.to_ascii_lowercase();
            let Some(bytes) = self.wad.read(wad_path_hash(&path))? else {
                continue;
            };
            if let Some(bytes) = crate::gear_toggle::add_toggle(&bytes, graph, &swaps)? {
                toggled = Some((path, bytes));
                break;
            }
        }
        let Some(graph_file) = toggled else {
            return Ok(None);
        };
        let mut files = vec![graph_file];
        let mut drivers = 0;
        let skin0 = match crate::gear_toggle::drive_by_parts(generated, &markers)? {
            Some((bytes, count)) => {
                drivers += count;
                bytes
            }
            None => generated.to_vec(),
        };
        let skin0 = crate::forms::strip_gear_indicators(&skin0)?;
        if let Some((bytes, count)) = crate::gear_toggle::drive_by_parts(source, &markers)? {
            drivers += count;
            files.push((source_path.to_owned(), bytes));
        }
        Ok(Some(FormCycle {
            files,
            skin0,
            forms: swaps.len(),
            drivers,
        }))
    }

    fn with_form_cycle(
        &self,
        source: &[u8],
        source_path: &str,
        skin: u32,
        generated: Vec<u8>,
        wad_root: &Path,
    ) -> Result<Vec<u8>, ClassicError> {
        match self.form_cycle(source, source_path, &generated) {
            Ok(Some(plan)) => {
                for (path, bytes) in &plan.files {
                    let target = wad_root.join(path);
                    if let Some(dir) = target.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::write(&target, bytes)?;
                }
                info!(
                    alias = %self.alias,
                    skin,
                    forms = plan.forms,
                    drivers = plan.drivers,
                    files = ?plan.files.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
                    "Ctrl+5 cycles the skin's forms in game"
                );
                Ok(plan.skin0)
            }
            Ok(None) => {
                debug!(alias = %self.alias, skin, "No in-game form cycling for this skin");
                Ok(generated)
            }
            Err(e) => {
                warn!(
                    alias = %self.alias,
                    skin,
                    error = %e,
                    "In-game form cycling not added; the skin keeps its first form"
                );
                Ok(generated)
            }
        }
    }

    fn skin_graph(&self, source: &[u8]) -> Result<Option<(String, u32)>, ClassicError> {
        let parsed = parse_prop_file(source).map_err(|e| ClassicError::Bin(e.to_string()))?;
        let Some(skin) = parsed
            .entries
            .iter()
            .find(|e| e.class_hash == SKIN_DATA_CLASS)
        else {
            return Ok(None);
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
            return Ok(None);
        };
        for link in parsed
            .links
            .iter()
            .filter(|l| l.to_ascii_lowercase().contains("/animations/"))
        {
            let path = link.to_ascii_lowercase();
            let holds = self
                .wad
                .read(wad_path_hash(&path))?
                .and_then(|bytes| parse_prop_file(&bytes).ok())
                .is_some_and(|bin| bin.entries.iter().any(|e| e.key_hash == graph));
            if holds {
                return Ok(Some((path, graph)));
            }
        }
        Ok(None)
    }

    fn missing_clips(
        &self,
        source: &[u8],
        wad_root: &Path,
    ) -> Result<Option<(String, crate::clip_alias::AliasedGraph)>, ClassicError> {
        let base_key = prop_key_hash(&format!("Characters/{}/Animations/Skin0", self.alias));
        let Some((path, graph)) = self.skin_graph(source)? else {
            return Ok(None);
        };
        if graph == base_key {
            return Ok(None);
        }
        let main = self.alias.to_ascii_lowercase();
        let Some(base) = self.wad.read(wad_path_hash(&format!(
            "data/characters/{main}/animations/skin0.bin"
        )))?
        else {
            return Ok(None);
        };
        let current = match std::fs::read(wad_root.join(&path)) {
            Ok(bytes) => bytes,
            Err(_) => match self.wad.read(wad_path_hash(&path))? {
                Some(bytes) => bytes,
                None => return Ok(None),
            },
        };
        let spells = self
            .wad
            .read(wad_path_hash(&format!("data/characters/{main}/{main}.bin")))?
            .map(|record| crate::clip_alias::spell_names(&record))
            .unwrap_or_default();
        if spells.is_empty() {
            return Ok(None);
        }
        Ok(
            crate::clip_alias::alias_missing_clips(&current, graph, &base, base_key, &spells)?
                .map(|aliased| (path, aliased)),
        )
    }

    fn with_missing_clips(
        &self,
        source: &[u8],
        skin: u32,
        wad_root: &Path,
    ) -> Result<(), ClassicError> {
        match self.missing_clips(source, wad_root) {
            Ok(Some((path, (bytes, aliases)))) => {
                let target = wad_root.join(&path);
                if let Some(dir) = target.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&target, bytes)?;
                info!(
                    alias = %self.alias,
                    skin,
                    graph = %path,
                    clips = ?aliases
                        .iter()
                        .map(|a| format!("{:08x}->{:08x} of {}", a.missing, a.variant, a.variants))
                        .collect::<Vec<_>>(),
                    "Clips the default skin's animations ask for now point at the skin's own version"
                );
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(e) => {
                warn!(
                    alias = %self.alias,
                    skin,
                    error = %e,
                    "Missing animation clips not aliased; the skin keeps the game's graph"
                );
                Ok(())
            }
        }
    }

    fn submeshes_of(&self, file: &PropFile, gear: &[u8]) -> Result<Vec<String>, ClassicError> {
        let mesh_path = |body: &[u8], path: &[&str]| {
            let hashes: Vec<u32> = path.iter().map(|p| prop_key_hash(p)).collect();
            field_value(body, &hashes).ok().flatten().and_then(|v| {
                v.bytes
                    .get(2..)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
            })
        };
        let skn =
            mesh_path(gear, &["mGearData", "skinMeshProperties", "simpleSkin"]).or_else(|| {
                file.entries
                    .iter()
                    .find(|e| e.class_hash == SKIN_DATA_CLASS)
                    .and_then(|skin| mesh_path(&skin.body, &["skinMeshProperties", "simpleSkin"]))
            });
        let Some(skn) = skn else {
            return Ok(Vec::new());
        };
        match self.wad.read(wad_path_hash(&skn.to_ascii_lowercase()))? {
            Some(bytes) => crate::forms::skn_submesh_names(&bytes),
            None => Ok(Vec::new()),
        }
    }

    fn build(
        &self,
        skin: u32,
        base_skin: Option<u32>,
        form: Option<u32>,
        mods_dir: &Path,
    ) -> Result<String, ClassicError> {
        let main = self.alias.to_ascii_lowercase();
        let target_bin = skin_bin(&main, skin);
        if !self.wad.contains(wad_path_hash(&target_bin)) {
            return Err(ClassicError::SkinNotFound {
                champion_id: 0,
                skin_id: skin,
            });
        }

        let game_parent = self.parent_skin(skin);
        if game_parent.is_some() && base_skin.is_some() && game_parent != base_skin {
            debug!(
                alias = %self.alias,
                skin,
                game_parent = ?game_parent,
                client_base = ?base_skin,
                "The game and the client name different parent skins; the game's is used"
            );
        }
        let base_skin = game_parent.or(base_skin);

        let folder = match form {
            Some(form) => format!("{STANDARD_MOD_PREFIX}{main}_{skin}_form{form}"),
            None => format!("{STANDARD_MOD_PREFIX}{main}_{skin}"),
        };
        let final_dir = mods_dir.join(&folder);
        let partial = mods_dir.join(format!("{folder}.partial"));
        remove_if_present(&partial)?;
        let wad_root = partial
            .join("WAD")
            .join(format!("{}.wad.client", self.alias));
        let characters_dir = wad_root.join("data").join("characters");

        let source = self
            .wad
            .read(wad_path_hash(&target_bin))?
            .ok_or_else(|| ClassicError::Bin(format!("{main} skin{skin}.bin not found in WAD")))?;
        let bins_dir = characters_dir.join(&main).join("skins");
        std::fs::create_dir_all(&bins_dir)?;
        let retargeted =
            retarget_skin_bin(&source, &self.alias, skin, 0, self.slot0_identity(&main))?;
        let retargeted = match form {
            Some(form) => self.bake_form(&source, retargeted, form)?,
            None => retargeted,
        };
        let retargeted =
            self.finish_slot0(&main, &self.alias, &source, retargeted, &characters_dir)?;
        let retargeted = if form.is_none() && !self.options.graph_in_slot0 {
            let retargeted =
                self.with_form_cycle(&source, &target_bin, skin, retargeted, &wad_root)?;
            self.with_missing_clips(&source, skin, &wad_root)?;
            retargeted
        } else {
            retargeted
        };
        let mut records = vec![generated_bin_record(
            &self.alias,
            &main,
            skin,
            &source,
            &retargeted,
        )];
        std::fs::write(bins_dir.join("skin0.bin"), retargeted)?;

        let mut retargeted_companions = Vec::new();
        for companion in self.companions() {
            let Some(source_skin) = self.companion_source_skin(&companion, skin, base_skin) else {
                continue;
            };
            let written = self
                .read_skin_bin(&companion, source_skin)
                .and_then(|source| {
                    source.ok_or_else(|| {
                        ClassicError::Bin(format!("{companion} skin{source_skin}.bin vanished"))
                    })
                })
                .and_then(|source| {
                    let retargeted = retarget_skin_bin(
                        &source,
                        &companion,
                        source_skin,
                        0,
                        self.slot0_identity(&companion),
                    )?;
                    let retargeted = self.finish_slot0(
                        &companion,
                        &companion,
                        &source,
                        retargeted,
                        &characters_dir,
                    )?;
                    records.push(generated_bin_record(
                        &self.alias,
                        &companion,
                        source_skin,
                        &source,
                        &retargeted,
                    ));
                    Ok(retargeted)
                })
                .and_then(|retargeted| {
                    let comp_dir = characters_dir.join(&companion).join("skins");
                    std::fs::create_dir_all(&comp_dir)?;
                    std::fs::write(comp_dir.join("skin0.bin"), retargeted)?;
                    Ok(())
                });
            if let Err(e) = written {
                warn!(
                    alias = %self.alias,
                    companion = %companion,
                    skin = source_skin,
                    error = %e,
                    "Companion skin not generated; it keeps its base look in this match"
                );
                continue;
            }
            retargeted_companions.push(companion);
        }

        let meta = partial.join("META");
        std::fs::create_dir_all(&meta)?;
        let info = serde_json::json!({
            "Author": "Dekan",
            "Name": format!("{} skin {skin}", self.alias),
            "Version": "1.0",
            "Description": "Generated dynamically from installed game WAD",
        });
        std::fs::write(meta.join("info.json"), info.to_string())?;
        let manifest = serde_json::json!({
            "alias": self.alias,
            "skin": skin,
            "base_skin": base_skin,
            "form": form,
            "game_wad": self.wad_stamp,
            "generated": records,
        });
        std::fs::write(
            meta.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|e| ClassicError::Bin(e.to_string()))?,
        )?;

        remove_if_present(&final_dir)?;
        std::fs::rename(&partial, &final_dir)?;

        info!(
            alias = %self.alias,
            skin,
            companions = ?retargeted_companions,
            folder = %folder,
            "Standard skin mod generated directly from installed game WAD"
        );

        Ok(folder)
    }
}

#[must_use]
pub fn resolve_alias_with_id(
    game_dir: &Path,
    client_alias: Option<&str>,
    champion_id: Option<u32>,
    library_champion_dir: &Path,
) -> Option<String> {
    let champions = game_dir.join("DATA").join("FINAL").join("Champions");
    if let Some(alias) = client_alias.filter(|a| is_safe_alias(a)) {
        if champions.join(format!("{alias}.wad.client")).is_file() {
            return Some(alias.to_owned());
        }
    }
    if let Some(installed) =
        champion_id.and_then(|id| crate::client_data::champion_alias(game_dir, id))
    {
        if is_safe_alias(&installed) && champions.join(format!("{installed}.wad.client")).is_file()
        {
            return Some(installed);
        }
    }
    resolve_alias(game_dir, client_alias, library_champion_dir)
}

#[must_use]
pub fn resolve_alias(
    game_dir: &Path,
    client_alias: Option<&str>,
    library_champion_dir: &Path,
) -> Option<String> {
    let champions = game_dir.join("DATA").join("FINAL").join("Champions");
    if let Some(alias) = client_alias.filter(|a| is_safe_alias(a)) {
        if champions.join(format!("{alias}.wad.client")).is_file() {
            return Some(alias.to_owned());
        }
        warn!(
            alias,
            "Client alias has no champion WAD; trying the skin library"
        );
    }

    let mut archives: Vec<PathBuf> = Vec::new();
    let mut stack = vec![library_champion_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("fantome") || e.eq_ignore_ascii_case("zip"))
            {
                archives.push(path);
            }
        }
    }
    archives.sort();

    for archive in archives {
        let Ok(file) = std::fs::File::open(&archive) else {
            continue;
        };
        match dekan_wad::fantome::wad_names_in_archive(std::io::BufReader::new(file)) {
            Ok(names) if names.len() == 1 => {
                if let Some(alias) = names.into_iter().next().filter(|a| is_safe_alias(a)) {
                    return Some(alias);
                }
            }
            Ok(names) => debug!(
                archive = %archive.display(),
                wads = names.len(),
                "Archive does not target exactly one champion WAD"
            ),
            Err(e) => debug!(archive = %archive.display(), error = %e, "Archive unreadable"),
        }
    }
    None
}

#[cfg(test)]
#[path = "generator_tests.rs"]
mod tests;
