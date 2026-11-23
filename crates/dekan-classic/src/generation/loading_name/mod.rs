use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, UNIX_EPOCH};

use dekan_wad::wad::WadFile;
use tracing::{debug, info, warn};

use crate::generator::StandardChampion;

mod records;
pub mod rst;

#[cfg(test)]
mod tests;

pub use records::{champion_aliases, ships_language_table};

pub const MOD_FOLDER: &str = "dekan_loading_name";

#[derive(Debug, Clone)]
pub struct LoadingNamePick {
    pub mod_folder: PathBuf,
    pub skin_id: u32,
    pub parent_skin_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    StringOverrides,
    LooseTable,
    Unsupported,
}

struct CachedTable {
    path: PathBuf,
    len: u64,
    modified: u64,
    bytes: Arc<Vec<u8>>,
}

static TABLE_CACHE: Mutex<Option<CachedTable>> = Mutex::new(None);

#[must_use]
pub fn client_locale(game_dir: &Path) -> Option<String> {
    let settings = game_dir
        .parent()?
        .join("Config")
        .join("LeagueClientSettings.yaml");
    let text = std::fs::read_to_string(settings).ok()?;
    let mut in_globals = false;
    let mut globals_indent = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line
            .bytes()
            .take_while(|&b| b == b' ' || b == b'\t')
            .count();
        if !in_globals {
            if trimmed == "globals:" {
                in_globals = true;
                globals_indent = indent;
            }
            continue;
        }
        if indent <= globals_indent {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("locale:") {
            let val = rest.trim().trim_matches('"').trim_matches('\'');
            let bytes = val.as_bytes();
            if bytes.len() == 5
                && bytes[2] == b'_'
                && bytes[0].is_ascii_alphabetic()
                && bytes[1].is_ascii_alphabetic()
                && bytes[3].is_ascii_alphabetic()
                && bytes[4].is_ascii_alphabetic()
            {
                return Some(val.to_owned());
            }
        }
    }
    None
}

#[must_use]
pub fn language_wad(game_dir: &Path) -> Option<PathBuf> {
    let wads = records::language_wads(game_dir);
    if let Some(locale) = client_locale(game_dir) {
        let target = format!("global.{}.wad.client", locale.to_ascii_lowercase());
        if let Some(found) = wads.iter().find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(&target))
        }) {
            return Some(found.clone());
        }
    }
    if wads.len() == 1 {
        return wads.into_iter().next();
    }
    None
}

#[must_use]
pub fn strategy_for(game_dir: &Path) -> Strategy {
    let locale = client_locale(game_dir)
        .unwrap_or_else(|| "en_us".to_owned())
        .to_ascii_lowercase();
    let has_en_us = records::language_wads(game_dir).iter().any(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("global.en_us.wad.client"))
    });
    if !has_en_us {
        return Strategy::LooseTable;
    }
    if locale == "en_us" {
        Strategy::StringOverrides
    } else {
        Strategy::Unsupported
    }
}

fn read_table_cached(wad_path: &Path) -> Option<Arc<Vec<u8>>> {
    let meta = std::fs::metadata(wad_path).ok()?;
    let len = meta.len();
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());

    if let Ok(guard) = TABLE_CACHE.lock()
        && let Some(cached) = guard.as_ref()
        && cached.path == wad_path
        && cached.len == len
        && cached.modified == modified
    {
        return Some(Arc::clone(&cached.bytes));
    }

    let wad = WadFile::open(wad_path).ok()?;
    let raw = wad.read(rst::table_hash()).ok()??;
    let arc = Arc::new(raw);
    if let Ok(mut guard) = TABLE_CACHE.lock() {
        *guard = Some(CachedTable {
            path: wad_path.to_path_buf(),
            len,
            modified,
            bytes: Arc::clone(&arc),
        });
    }
    Some(arc)
}

fn resolve_skin_display_name(
    game_dir: &Path,
    table: &[u8],
    alias: &str,
    skin_id: u32,
    parent_skin_id: Option<u32>,
) -> Option<String> {
    let skin_num = skin_id % 1000;
    let parent_num = parent_skin_id
        .map(|id| id % 1000)
        .filter(|&n| n != 0 && n != skin_num)
        .or_else(|| {
            StandardChampion::open(game_dir, alias)
                .ok()
                .and_then(|champ| champ.parent_skin(skin_num))
        });

    let mut candidates = Vec::with_capacity(2);
    if let Some(parent) = parent_num {
        candidates.push(parent);
    }
    if skin_num != 0 && !candidates.contains(&skin_num) {
        candidates.push(skin_num);
    }

    for num in candidates {
        let key = format!("game_character_skin_displayname_{alias}_{num}");
        if let Some(name) = rst::read_text(table, rst::key_hash(&key))
            && !name.trim().is_empty()
        {
            return Some(name);
        }
    }
    None
}

fn remove_dir_quiet(path: &Path) {
    if path.exists() {
        let _ = std::fs::remove_dir_all(path); // ignore-ok: best-effort cleanup of stale loading name mod
    }
}

pub fn build_loading_name_mod(
    game_dir: &Path,
    mods_dir: &Path,
    picks: &[LoadingNamePick],
    other_mods: &[PathBuf],
) -> Option<String> {
    let started = Instant::now();
    let final_dir = mods_dir.join(MOD_FOLDER);
    let active_picks: Vec<&LoadingNamePick> = picks
        .iter()
        .filter(|p| p.skin_id != 0 && p.skin_id % 1000 != 0)
        .collect();
    if active_picks.is_empty() {
        remove_dir_quiet(&final_dir);
        return None;
    }

    let strategy = strategy_for(game_dir);
    if strategy == Strategy::Unsupported {
        info!(
            "Install has Global.en_US.wad.client alongside another locale; keeping champion name on loading screen"
        );
        remove_dir_quiet(&final_dir);
        return None;
    }

    if strategy == Strategy::LooseTable
        && other_mods.iter().any(|other| ships_language_table(other))
    {
        info!(
            "Another active mod carries its own lol.stringtable; keeping champion name on loading screen"
        );
        remove_dir_quiet(&final_dir);
        return None;
    }

    let Some(wad_path) = language_wad(game_dir) else {
        debug!("Game language WAD could not be resolved; skipping loading screen skin name");
        remove_dir_quiet(&final_dir);
        return None;
    };
    let Some(table) = read_table_cached(&wad_path) else {
        warn!(wad = %wad_path.display(), "Could not read lol.stringtable from language WAD");
        remove_dir_quiet(&final_dir);
        return None;
    };

    let partial = mods_dir.join(format!("{MOD_FOLDER}.partial"));
    remove_dir_quiet(&partial);
    if std::fs::create_dir_all(partial.join("META")).is_err() {
        return None;
    }

    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    let mut descriptions: Vec<String> = Vec::new();
    let mut total_repointed = 0usize;

    for pick in active_picks {
        for alias in champion_aliases(&pick.mod_folder) {
            let Some(skin_display) = resolve_skin_display_name(
                game_dir,
                &table,
                &alias,
                pick.skin_id,
                pick.parent_skin_id,
            ) else {
                continue;
            };
            let champ_key = format!("{}{alias}", rst::RECORD_KEY_PREFIX);
            let Some(champ_display) = rst::read_text(&table, rst::key_hash(&champ_key)) else {
                continue;
            };

            let repointed =
                records::repoint_champion_records(game_dir, &pick.mod_folder, &alias, &partial);
            total_repointed += repointed;
            texts.insert(champ_key, skin_display.clone());
            if repointed > 0 {
                texts.insert(
                    format!("{}{alias}", rst::RECORD_KEY_OWN_PREFIX),
                    champ_display,
                );
            }
            descriptions.push(skin_display);
            break;
        }
    }

    if texts.is_empty() {
        remove_dir_quiet(&partial);
        remove_dir_quiet(&final_dir);
        return None;
    }

    let description = descriptions.join(", ");
    let info_json = match strategy {
        Strategy::StringOverrides => serde_json::json!({
            "Author": "Dekan",
            "Description": description,
            "Name": "Loading screen name",
            "Version": "1.0.0",
            "Layers": {
                "base": {
                    "Name": "base",
                    "Priority": 0,
                    "StringOverrides": {
                        "en_us": texts
                    }
                }
            }
        }),
        Strategy::LooseTable => {
            let updates: Vec<(String, String)> = texts.into_iter().collect();
            let Some(patched_table) = rst::with_texts(&table, &updates) else {
                remove_dir_quiet(&partial);
                return None;
            };
            let table_dst = partial
                .join("RAW")
                .join("DATA")
                .join("Menu")
                .join("en_US")
                .join("lol.stringtable");
            if let Some(parent) = table_dst.parent()
                && std::fs::create_dir_all(parent).is_err()
            {
                remove_dir_quiet(&partial);
                return None;
            }
            if std::fs::write(&table_dst, patched_table).is_err() {
                remove_dir_quiet(&partial);
                return None;
            }
            serde_json::json!({
                "Author": "Dekan",
                "Description": description,
                "Name": "Loading screen name",
                "Version": "1.0.0"
            })
        }
        Strategy::Unsupported => {
            remove_dir_quiet(&partial);
            return None;
        }
    };

    let Ok(info_bytes) = serde_json::to_vec(&info_json) else {
        remove_dir_quiet(&partial);
        return None;
    };
    if std::fs::write(partial.join("META").join("info.json"), info_bytes).is_err() {
        remove_dir_quiet(&partial);
        return None;
    }

    remove_dir_quiet(&final_dir);
    if let Err(e) = std::fs::rename(&partial, &final_dir) {
        warn!(error = %e, "Could not finalize loading screen skin name mod");
        remove_dir_quiet(&partial);
        return None;
    }

    info!(
        names = ?descriptions,
        strategy = ?strategy,
        repointed_records = total_repointed,
        elapsed_ms = started.elapsed().as_millis(),
        "Loading screen skin name mod built"
    );
    Some(MOD_FOLDER.to_owned())
}
