use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use dekan_classic::client_data::ClientGameData;
use dekan_classic::generator::{ClassicChampion, StandardChampion, slots_for};
use dekan_inject::overlay_builder;
use dekan_lcu::champion_assets::ChampionAssets;
use dekan_wad::wad::WadFile;

#[derive(Debug, Clone)]
pub enum Case {
    Standard {
        alias: String,
        skin: u32,
        base: Option<u32>,
        label: String,
    },
    Classic {
        alias: String,
        classic_alias: String,
        skin: u32,
        label: String,
    },
}

impl Case {
    fn label(&self) -> &str {
        match self {
            Self::Standard { label, .. } | Self::Classic { label, .. } => label,
        }
    }
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub label: String,
    pub failures: Vec<String>,
    pub overlay_wads: usize,
    pub changed_entries: usize,
    pub verbatim_entries: usize,
    pub build_ms: u128,
}

fn assets_of(data: &ClientGameData, id: u32) -> Option<ChampionAssets> {
    data.read(&format!("v1/champions/{id}.json"))
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

pub fn sample(data: &ClientGameData, game: &Path) -> Result<Vec<Case>, String> {
    let aliases = data.champion_aliases().map_err(|e| e.to_string())?;
    let mut cases = Vec::new();
    for (id, alias) in aliases.iter().filter(|(id, _)| **id < 60_000) {
        let Some(assets) = assets_of(data, *id) else {
            continue;
        };
        let Ok(champion) = StandardChampion::open(game, alias) else {
            continue;
        };
        let offered: Vec<_> = assets
            .skins
            .iter()
            .filter(|s| !s.is_base && champion.has_skin(s.id % 1000))
            .collect();
        if let Some(latest) = offered.iter().max_by_key(|s| s.id % 1000) {
            cases.push(Case::Standard {
                alias: alias.clone(),
                skin: latest.id % 1000,
                base: None,
                label: format!("{alias} {} {}", latest.id, latest.name),
            });
        }
        if let Some((skin, chroma)) = offered
            .iter()
            .rev()
            .find_map(|s| s.chromas.last().map(|c| (s, c)))
            .filter(|(_, c)| champion.has_skin(c.id % 1000))
        {
            cases.push(Case::Standard {
                alias: alias.clone(),
                skin: chroma.id % 1000,
                base: Some(skin.id % 1000),
                label: format!("{alias} {} {}", chroma.id, chroma.name),
            });
        }
    }
    for (alias, skin, base, label) in [
        ("Orianna", 1, None, "reported: Orianna ball"),
        ("Zed", 10, None, "reported: Zed shadows, Galaxy Slayer"),
        ("Zed", 12, Some(10), "reported: Zed chroma 238012"),
        ("Seraphine", 2, Some(1), "tier: K/DA ALL OUT Rising Star"),
        ("Seraphine", 3, Some(1), "tier: K/DA ALL OUT Superstar"),
        (
            "Tristana",
            80,
            Some(79),
            "tier: Immortalized Legend Tristana",
        ),
        ("Ahri", 86, Some(85), "tier: Immortalized Legend Ahri"),
        ("Kaisa", 71, Some(70), "tier: Immortalized Legend Kai'Sa"),
        (
            "Garen",
            44,
            None,
            "legendary animation graph: God-King Garen",
        ),
        ("Lux", 7, None, "ultimate: Elementalist Lux"),
    ] {
        cases.push(Case::Standard {
            alias: alias.into(),
            skin,
            base,
            label: label.into(),
        });
    }
    for (alias, classic_alias, skin, label) in [
        (
            "Annie",
            "Jade_Annie",
            303,
            "classic: Classic Annie (Founder's Goth)",
        ),
        (
            "MonkeyKing",
            "Jade_Wukong",
            3,
            "classic: Jade Dragon Wukong",
        ),
    ] {
        cases.push(Case::Classic {
            alias: alias.into(),
            classic_alias: classic_alias.into(),
            skin,
            label: label.into(),
        });
    }
    Ok(cases)
}

fn generate(case: &Case, game: &Path, mods: &Path, cache: &Path) -> Result<String, String> {
    match case {
        Case::Standard {
            alias, skin, base, ..
        } => StandardChampion::open(game, alias)
            .map_err(|e| e.to_string())?
            .with_cache_dir(cache)
            .build_mod(*skin, *base, mods)
            .map_err(|e| e.to_string()),
        Case::Classic {
            alias,
            classic_alias,
            skin,
            ..
        } => {
            let champion = ClassicChampion::open(game, alias)
                .map_err(|e| e.to_string())?
                .with_client_character(Some(classic_alias));
            let known = champion.jade_names_from_bins_cached(cache);
            champion
                .build_mod(*skin, &slots_for(None), &known, mods)
                .map_err(|e| e.to_string())
        }
    }
}

fn overlay_wads(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".wad.client"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

pub fn run_case(case: &Case, game: &Path, scratch: &Path, cache: &Path) -> Outcome {
    let mut outcome = Outcome {
        label: case.label().to_owned(),
        ..Outcome::default()
    };
    let mods = scratch.join("mods");
    let overlay = scratch.join("overlay");
    let _ = std::fs::remove_dir_all(scratch); // ignore-ok: scratch folder may not exist yet
    if let Err(e) = std::fs::create_dir_all(&mods) {
        outcome.failures.push(format!("scratch folder: {e}"));
        return outcome;
    }

    let folder = match generate(case, game, &mods, cache) {
        Ok(folder) => folder,
        Err(e) => {
            outcome.failures.push(format!("generation: {e}"));
            return outcome;
        }
    };
    let started = std::time::Instant::now();
    if let Err(e) = overlay_builder::build(
        game,
        &mods,
        &overlay,
        std::slice::from_ref(&folder),
        &AtomicBool::new(false),
    ) {
        outcome.failures.push(format!("overlay build: {e}"));
        return outcome;
    }
    outcome.build_ms = started.elapsed().as_millis();

    let index = match overlay_builder::get_or_index_game(game) {
        Ok(index) => index,
        Err(e) => {
            outcome.failures.push(format!("game index: {e}"));
            return outcome;
        }
    };
    let rewritten: Vec<PathBuf> = overlay_wads(&overlay);
    outcome.overlay_wads = rewritten.len();
    if rewritten.is_empty() {
        outcome.failures.push("the overlay holds no WAD".into());
        return outcome;
    }
    let rewritten_rel: Vec<PathBuf> = rewritten
        .iter()
        .filter_map(|p| p.strip_prefix(&overlay).ok().map(Path::to_path_buf))
        .collect();

    let mut changed: BTreeMap<u64, (PathBuf, Vec<u8>)> = BTreeMap::new();
    for (overlay_wad, relative) in rewritten.iter().zip(&rewritten_rel) {
        let built = match WadFile::open(overlay_wad) {
            Ok(wad) => wad,
            Err(e) => {
                outcome
                    .failures
                    .push(format!("{}: opens: {e}", relative.display()));
                continue;
            }
        };
        let original = match WadFile::open(&game.join(relative)) {
            Ok(wad) => wad,
            Err(e) => {
                outcome
                    .failures
                    .push(format!("{}: game copy: {e}", relative.display()));
                continue;
            }
        };
        let game_entries: HashMap<u64, _> = original.toc().map(|e| (e.path_hash, e)).collect();
        for entry in built.toc() {
            if let Some(game_entry) = game_entries.get(&entry.path_hash) {
                if game_entry.compression == entry.compression
                    && game_entry.compressed_size == entry.compressed_size
                    && game_entry.uncompressed_size == entry.uncompressed_size
                    && game_entry.checksum == entry.checksum
                {
                    outcome.verbatim_entries += 1;
                    continue;
                }
            }
            let decoded = match built.read(entry.path_hash) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    outcome.failures.push(format!(
                        "{}: {:#x} vanished",
                        relative.display(),
                        entry.path_hash
                    ));
                    continue;
                }
                Err(e) => {
                    outcome.failures.push(format!(
                        "{}: {:#x} does not decode: {e}",
                        relative.display(),
                        entry.path_hash
                    ));
                    continue;
                }
            };
            match game_entries.get(&entry.path_hash) {
                Some(game_entry)
                    if original.read(entry.path_hash).ok().flatten().as_deref()
                        == Some(&decoded[..]) =>
                {
                    outcome.failures.push(format!(
                        "{}: {:#x} unchanged but re-encoded ({:?}/{} -> {:?}/{})",
                        relative.display(),
                        entry.path_hash,
                        game_entry.compression,
                        game_entry.compressed_size,
                        entry.compression,
                        entry.compressed_size
                    ));
                }
                _ => {
                    changed.insert(entry.path_hash, (relative.clone(), decoded));
                }
            }
        }
        for hash in game_entries.keys() {
            if built.entry(*hash).is_none() {
                outcome.failures.push(format!(
                    "{}: {hash:#x} dropped from the game's WAD",
                    relative.display()
                ));
            }
        }
    }
    outcome.changed_entries = changed.len();
    if changed.is_empty() {
        outcome
            .failures
            .push("no entry differs from the game: the skin would not change".into());
    }

    for (hash, (owner, bytes)) in &changed {
        for wad in index.values().filter(|w| w.contains(*hash)) {
            if rewritten_rel.iter().any(|r| r == &wad.relpath) || &wad.relpath == owner {
                continue;
            }
            let game_bytes = WadFile::open(&wad.path)
                .ok()
                .and_then(|w| w.read(*hash).ok().flatten());
            if game_bytes.as_deref() != Some(&bytes[..]) {
                outcome.failures.push(format!(
                    "{hash:#x} changed in {} but {} (mounted as the game has it) holds other bytes: Inconsistent",
                    owner.display(),
                    wad.relpath.display()
                ));
            }
        }
    }
    let _ = std::fs::remove_dir_all(scratch); // ignore-ok: scratch folder
    outcome
}

#[must_use]
pub fn render(outcomes: &[Outcome]) -> String {
    let failed: Vec<&Outcome> = outcomes.iter().filter(|o| !o.failures.is_empty()).collect();
    let mut lines = vec![
        "# Pipeline harness".to_owned(),
        String::new(),
        format!(
            "- cases: {} | passed: {} | failed: {}",
            outcomes.len(),
            outcomes.len() - failed.len(),
            failed.len()
        ),
        format!(
            "- overlay build time: median {} ms, max {} ms",
            median(outcomes.iter().map(|o| o.build_ms).collect()),
            outcomes.iter().map(|o| o.build_ms).max().unwrap_or(0)
        ),
        String::new(),
        "| Case | Result | Overlay WADs | Changed | Verbatim | Build ms |".to_owned(),
        "| --- | --- | --- | --- | --- | --- |".to_owned(),
    ];
    for o in outcomes {
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            o.label,
            if o.failures.is_empty() { "ok" } else { "FAIL" },
            o.overlay_wads,
            o.changed_entries,
            o.verbatim_entries,
            o.build_ms
        ));
    }
    lines.extend([String::new(), "## Failures".to_owned(), String::new()]);
    for o in &failed {
        for failure in &o.failures {
            lines.push(format!("- {}: {failure}", o.label));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn median(mut values: Vec<u128>) -> u128 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}
