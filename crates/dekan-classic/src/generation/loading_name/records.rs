use std::path::{Path, PathBuf};

use dekan_wad::hash::wad_path_hash;
use dekan_wad::wad::WadFile;

use super::rst;

fn form_records(alias: &str) -> &'static [&'static str] {
    match alias.to_ascii_lowercase().as_str() {
        "cassiopeia" => &["cassiopeia_death"],
        "elise" => &["elisespider"],
        "fiddlesticks" => &["fiddlestickseffigy"],
        "fizz" => &["fizzbait", "fizzshark", "jade_fizzbait", "jade_fizzshark"],
        "kogmaw" => &["jade_kogmawdead", "kogmawdead"],
        "locke" => &["locketotem"],
        "lux" => &[
            "luxair",
            "luxdark",
            "luxfire",
            "luxice",
            "luxmagma",
            "luxmystic",
            "luxnature",
            "luxstorm",
            "luxwater",
        ],
        "monkeyking" => &["monkeykingflying"],
        "nasus" => &["jade_nasusult", "nasusult"],
        "nidalee" => &["jade_nidaleecougar", "nidaleecougar"],
        "orianna" => &["oriannaball", "oriannanoball"],
        "quinn" => &["quinnvalor"],
        "rammus" => &["jade_rammusdbc", "jade_rammuspb", "rammusdbc", "rammuspb"],
        "shyvana" => &["jade_shyvanadragon", "shyvanadragon"],
        "sona" => &["sonadjgenre01", "sonadjgenre02", "sonadjgenre03"],
        "swain" => &["jade_swaindemonform", "swaindemonform"],
        "syndra" => &["syndraorbs", "syndrasphere"],
        "zed" => &["zedshadow"],
        "zoe" => &["zoeorbs"],
        _ => &[],
    }
}

pub(super) fn language_wads(game_dir: &Path) -> Vec<PathBuf> {
    let localized = game_dir.join("DATA").join("FINAL").join("Localized");
    let Ok(entries) = std::fs::read_dir(localized) else {
        return Vec::new();
    };
    let mut wads: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    let lower = n.to_ascii_lowercase();
                    lower.starts_with("global.")
                        && lower.ends_with(".wad.client")
                        && lower != "global.wad.client"
                })
        })
        .collect();
    wads.sort();
    wads
}

fn dir_contains_stringtable(dir: &Path, table_hex: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if dir_contains_stringtable(&path, table_hex) {
                return true;
            }
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_ascii_lowercase();
            if lower == "lol.stringtable" || lower == format!("{table_hex}.stringtable") {
                return true;
            }
        }
    }
    false
}

#[must_use]
pub fn ships_language_table(mod_dir: &Path) -> bool {
    let hash = rst::table_hash();
    let hex = format!("{hash:016x}");
    for raw_name in ["RAW", "raw"] {
        let raw_dir = mod_dir.join(raw_name);
        if raw_dir.is_dir() && dir_contains_stringtable(&raw_dir, &hex) {
            return true;
        }
    }
    for wad_name in ["WAD", "wad"] {
        let wad_dir = mod_dir.join(wad_name);
        let Ok(entries) = std::fs::read_dir(&wad_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if !lower.starts_with("global.") || !lower.ends_with(".wad.client") {
                continue;
            }
            if path.is_dir() && dir_contains_stringtable(&path, &hex) {
                return true;
            }
            if path.is_file()
                && let Ok(wad) = WadFile::open(&path)
                && wad.contains(hash)
            {
                return true;
            }
        }
    }
    false
}

#[must_use]
pub fn champion_aliases(mod_dir: &Path) -> Vec<String> {
    let mut aliases = Vec::new();
    for wad_name in ["WAD", "wad"] {
        let wad_dir = mod_dir.join(wad_name);
        let Ok(entries) = std::fs::read_dir(&wad_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            let Some(stem) = lower.strip_suffix(".wad.client") else {
                continue;
            };
            let prefix_len = stem.find('.').unwrap_or(stem.len());
            let alias = &name[..prefix_len];
            if alias.is_empty() || alias.eq_ignore_ascii_case("global") {
                continue;
            }
            if !aliases
                .iter()
                .any(|a: &String| a.eq_ignore_ascii_case(alias))
            {
                aliases.push(alias.to_owned());
            }
        }
    }
    aliases
}

fn record_rel_path(character: &str) -> String {
    let lower = character.to_ascii_lowercase();
    format!("data/characters/{lower}/{lower}.bin")
}

fn read_character_record(
    game_dir: &Path,
    mod_dir: &Path,
    alias: &str,
    character: &str,
    installed_wad: Option<&WadFile>,
) -> Option<Vec<u8>> {
    let rel = record_rel_path(character);
    let hash = wad_path_hash(&rel);
    for wad_name in ["WAD", "wad"] {
        let candidate = mod_dir.join(wad_name).join(format!("{alias}.wad.client"));
        if candidate.is_file()
            && let Ok(wad) = WadFile::open(&candidate)
            && let Ok(Some(bytes)) = wad.read(hash)
        {
            return Some(bytes);
        }
        if candidate.is_dir() {
            let loose = candidate.join(&rel);
            if loose.is_file()
                && let Ok(bytes) = std::fs::read(&loose)
            {
                return Some(bytes);
            }
            let hashed = candidate.join(format!("{hash:016x}.bin"));
            if hashed.is_file()
                && let Ok(bytes) = std::fs::read(&hashed)
            {
                return Some(bytes);
            }
        }
    }
    if let Some(wad) = installed_wad
        && let Ok(Some(bytes)) = wad.read(hash)
    {
        return Some(bytes);
    }
    let fallback = game_dir
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join(format!("{alias}.wad.client"));
    WadFile::open(&fallback).ok()?.read(hash).ok()?
}

pub(super) fn repoint_champion_records(
    game_dir: &Path,
    mod_dir: &Path,
    alias: &str,
    partial_dir: &Path,
) -> usize {
    let installed_path = game_dir
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .join(format!("{alias}.wad.client"));
    let installed_wad = WadFile::open(&installed_path).ok();
    let lower_alias = alias.to_ascii_lowercase();
    let mut repointed = 0usize;
    for character in
        std::iter::once(lower_alias.as_str()).chain(form_records(alias).iter().copied())
    {
        let Some(raw) =
            read_character_record(game_dir, mod_dir, alias, character, installed_wad.as_ref())
        else {
            continue;
        };
        let Some(patched) = rst::patch_record_key(&raw, alias) else {
            continue;
        };
        let dst = partial_dir.join("RAW").join(record_rel_path(character));
        if let Some(parent) = dst.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            continue;
        }
        if std::fs::write(&dst, patched).is_ok() {
            repointed += 1;
        }
    }
    repointed
}
