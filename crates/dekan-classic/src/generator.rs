use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use dekan_wad::hash::{prop_key_hash, wad_path_hash};
use dekan_wad::prop::{PropEntry, PropFile, parse_prop_file, serialize_prop_file};
use dekan_wad::wad::WadFile;
use tracing::{debug, info, warn};

use crate::builder::CLASSIC_DEFAULT_SLOTS;
use crate::error::ClassicError;

pub const CLASSIC_MOD_PREFIX: &str = "classic_";

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

pub fn retarget_skin_bin(
    source: &[u8],
    character: &str,
    source_skin: u32,
    target_skin: u32,
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

    let selected: Vec<PropEntry> = parsed
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

    if !selected.iter().any(|e| e.key_hash == skin_target_hash) {
        return Err(ClassicError::Bin(format!(
            "{source_prefix} object not found in the skin bin"
        )));
    }

    // The skin object keeps pointing at its own animation graph and VFX by hash; those objects
    // live in the source bin's dependencies (`Animations/SkinN.bin`, shared skin bins). Dropping
    // them left legendary/mythic skins on the base graph: the model loaded but abilities froze.
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

pub struct ClassicChampion {
    alias: String,
    wad: WadFile,

    wad_stamp: String,
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
        let wad_stamp = std::fs::metadata(&path)
            .map(|meta| {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                format!("{}:{mtime}", meta.len())
            })
            .unwrap_or_default();
        Ok(Self {
            alias: alias.to_owned(),
            wad: WadFile::open(&path)?,
            wad_stamp,
        })
    }

    #[must_use]
    pub fn jade_names_in_bins(&self) -> BTreeSet<String> {
        const MAX_BIN_BYTES: usize = 8 * 1024 * 1024;
        const NEEDLE: &[u8] = b"characters/jade_";

        let mut found = BTreeSet::new();
        let mut unreadable = 0usize;
        for (hash, size) in self.wad.entries() {
            if size > MAX_BIN_BYTES {
                continue;
            }
            let bytes = match self.wad.read(hash) {
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
                let name_start = from + pos + b"characters/".len();
                let name: Vec<u8> = lower[name_start..]
                    .iter()
                    .copied()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    .collect();
                if lower.get(name_start + name.len()) == Some(&b'/') {
                    if let Ok(name) = String::from_utf8(name) {
                        found.insert(name);
                    }
                }
                from = name_start;
            }
        }
        if unreadable > 0 {
            warn!(
                alias = %self.alias,
                unreadable,
                "Some entries of the champion WAD could not be read while looking for Jade characters"
            );
        }
        found
    }

    #[must_use]
    pub fn jade_names_from_bins_cached(&self, cache_dir: &Path) -> BTreeSet<String> {
        let cache_path = cache_dir.join(format!(
            "classic_bin_names_{}.json",
            self.alias.to_ascii_lowercase()
        ));
        if !self.wad_stamp.is_empty() {
            if let Ok(bytes) = std::fs::read(&cache_path) {
                if let Ok(cached) = serde_json::from_slice::<CharacterCache>(&bytes) {
                    if cached.source == self.wad_stamp {
                        debug!(
                            alias = %self.alias,
                            characters = cached.characters.len(),
                            "Rift Classic character names from the bin-scan cache"
                        );
                        return cached.characters;
                    }
                }
            }
        }

        let started = std::time::Instant::now();
        let characters = self.jade_names_in_bins();
        info!(
            alias = %self.alias,
            names = ?characters,
            elapsed_ms = started.elapsed().as_millis(),
            "Rift Classic character names recovered from the champion's bins (no hash table)"
        );
        if !self.wad_stamp.is_empty() {
            let cache = CharacterCache {
                source: self.wad_stamp.clone(),
                characters: characters.clone(),
            };
            match serde_json::to_vec(&cache) {
                Ok(bytes) => {
                    if let Err(e) = std::fs::write(&cache_path, bytes) {
                        debug!(error = %e, "Rift Classic bin-scan cache not written; the next build scans again");
                    }
                }
                Err(e) => {
                    debug!(error = %e, "Rift Classic bin-scan cache not serialized");
                }
            }
        }
        characters
    }

    #[must_use]
    pub fn present_characters(&self, known: &BTreeSet<String>) -> Vec<String> {
        let mut candidates: BTreeSet<String> = known.clone();
        candidates.insert(main_character(&self.alias));
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
        let main = main_character(&self.alias);
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
                format!("Jade_{}", self.alias)
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
                let bin = retarget_skin_bin(&source, &display, skin, slot)?;
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

#[derive(Debug)]
pub struct StandardChampion {
    pub alias: String,
    wad: dekan_wad::wad::WadFile,
}

impl StandardChampion {
    pub fn open(game_dir: &Path, alias: &str) -> Result<Self, ClassicError> {
        let wad_path = game_dir
            .join("DATA")
            .join("FINAL")
            .join("Champions")
            .join(format!("{alias}.wad.client"));
        let wad = dekan_wad::wad::WadFile::open(&wad_path).map_err(ClassicError::Wad)?;
        Ok(Self {
            alias: alias.to_owned(),
            wad,
        })
    }

    #[must_use]
    pub fn has_skin(&self, skin: u32) -> bool {
        let main = self.alias.to_ascii_lowercase();
        self.wad.contains(wad_path_hash(&skin_bin(&main, skin)))
    }

    #[must_use]
    pub fn has_animation(&self, skin: u32) -> bool {
        let main = self.alias.to_ascii_lowercase();
        self.wad
            .contains(wad_path_hash(&animation_bin(&main, skin)))
    }

    #[must_use]
    pub fn skin_numbers(&self, limit: u32) -> Vec<u32> {
        (0..limit).filter(|n| self.has_skin(*n)).collect()
    }

    pub fn build_mod(&self, skin: u32, mods_dir: &Path) -> Result<String, ClassicError> {
        let main = self.alias.to_ascii_lowercase();
        let target_bin = skin_bin(&main, skin);
        if !self.wad.contains(wad_path_hash(&target_bin)) {
            return Err(ClassicError::SkinNotFound {
                champion_id: 0,
                skin_id: skin,
            });
        }

        let folder = format!(
            "{STANDARD_MOD_PREFIX}{}_{skin}",
            self.alias.to_ascii_lowercase()
        );
        let final_dir = mods_dir.join(&folder);
        let partial = mods_dir.join(format!("{folder}.partial"));
        remove_if_present(&partial)?;

        let source = self
            .wad
            .read(wad_path_hash(&target_bin))?
            .ok_or_else(|| ClassicError::Bin(format!("{main} skin{skin}.bin not found in WAD")))?;

        let bins_dir = partial
            .join("WAD")
            .join(format!("{}.wad.client", self.alias))
            .join("data")
            .join("characters")
            .join(&main)
            .join("skins");
        std::fs::create_dir_all(&bins_dir)?;

        let retargeted = retarget_skin_bin(&source, &self.alias, skin, 0)?;
        std::fs::write(bins_dir.join("skin0.bin"), retargeted)?;

        let target_anim = animation_bin(&main, skin);
        if self.wad.contains(wad_path_hash(&target_anim)) {
            match self.wad.read(wad_path_hash(&target_anim)) {
                Ok(Some(anim_source)) => {
                    match retarget_animation_bin(&anim_source, &self.alias, skin, 0) {
                        Ok(retargeted_anim) => {
                            let anim_dir = partial
                                .join("WAD")
                                .join(format!("{}.wad.client", self.alias))
                                .join("data")
                                .join("characters")
                                .join(&main)
                                .join("animations");
                            std::fs::create_dir_all(&anim_dir)?;
                            std::fs::write(anim_dir.join("skin0.bin"), retargeted_anim)?;
                            debug!(
                                alias = %self.alias,
                                skin,
                                "Custom animation graph retargeted to slot 0 for legendary/mythic skin"
                            );
                        }
                        Err(e) => {
                            warn!(
                                alias = %self.alias,
                                skin,
                                error = %e,
                                "Failed to retarget custom animation bin; falling back to base animation"
                            );
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(
                        alias = %self.alias,
                        skin,
                        error = %e,
                        "Failed to read custom animation bin from WAD"
                    );
                }
            }
        }

        let companions = dekan_core::champions::companion_characters(&self.alias);
        for companion in companions {
            let comp_bin = skin_bin(companion, skin);
            if !self.wad.contains(wad_path_hash(&comp_bin)) {
                continue;
            }
            let written = self
                .wad
                .read(wad_path_hash(&comp_bin))
                .map_err(ClassicError::from)
                .and_then(|source| {
                    source.ok_or_else(|| {
                        ClassicError::Bin(format!("{companion} skin{skin}.bin vanished"))
                    })
                })
                .and_then(|source| retarget_skin_bin(&source, companion, skin, 0))
                .and_then(|retargeted| {
                    let comp_dir = partial
                        .join("WAD")
                        .join(format!("{}.wad.client", self.alias))
                        .join("data")
                        .join("characters")
                        .join(companion)
                        .join("skins");
                    std::fs::create_dir_all(&comp_dir)?;
                    std::fs::write(comp_dir.join("skin0.bin"), retargeted)?;
                    Ok(())
                });
            if let Err(e) = written {
                warn!(
                    alias = %self.alias,
                    companion,
                    skin,
                    error = %e,
                    "Companion skin not generated; it keeps its base look in this match"
                );
            }

            let comp_anim = animation_bin(companion, skin);
            if self.wad.contains(wad_path_hash(&comp_anim)) {
                let _ = self // ignore-ok: companion animation fallback
                    .wad
                    .read(wad_path_hash(&comp_anim))
                    .map_err(ClassicError::from)
                    .and_then(|source| {
                        source.ok_or_else(|| {
                            ClassicError::Bin(format!("{companion} anim skin{skin}.bin vanished"))
                        })
                    })
                    .and_then(|source| retarget_animation_bin(&source, companion, skin, 0))
                    .and_then(|retargeted| {
                        let comp_anim_dir = partial
                            .join("WAD")
                            .join(format!("{}.wad.client", self.alias))
                            .join("data")
                            .join("characters")
                            .join(companion)
                            .join("animations");
                        std::fs::create_dir_all(&comp_anim_dir)?;
                        std::fs::write(comp_anim_dir.join("skin0.bin"), retargeted)?;
                        Ok(())
                    });
            }
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

        remove_if_present(&final_dir)?;
        std::fs::rename(&partial, &final_dir)?;

        info!(
            alias = %self.alias,
            skin,
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
    if let Some(id) = champion_id {
        if let Some(registered) = dekan_core::champions::champion_alias_by_id(id) {
            if champions.join(format!("{registered}.wad.client")).is_file() {
                return Some(registered.to_owned());
            }
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
