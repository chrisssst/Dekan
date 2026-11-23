use std::path::{Path, PathBuf};

use dekan_wad::hash::{prop_key_hash, wad_path_hash};
use dekan_wad::prop::{parse_prop_file, serialize_prop_file};
use dekan_wad::wad::WadFile;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::ClassicError;

pub const TFT_MOD_FOLDER: &str = "dekan_tft_cosmetics";
pub const DEFAULT_TACTICIAN_DONOR: &str = "data/characters/pettftavatar/skins/skin1.bin";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TftCategory {
    Tactician,
    Boom,
    Portal,
    Arena,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TftSwap {
    pub donor_id: u32,
    pub donor_path: String,
    pub target_id: u32,
    pub target_path: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TftLoadout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tactician: Option<TftSwap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boom: Option<TftSwap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal: Option<TftSwap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arena: Option<TftSwap>,
}

impl TftLoadout {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tactician.is_none()
            && self.boom.is_none()
            && self.portal.is_none()
            && self.arena.is_none()
    }
}

fn normalize_asset_path(raw: &str) -> Option<String> {
    let clean = raw
        .trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    if clean.is_empty()
        || clean.contains("..")
        || !clean.starts_with("data/")
        || !clean
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
    {
        return None;
    }
    Some(clean)
}

fn prop_record_key_from_bin_path(bin_path: &str) -> Option<String> {
    let no_data = bin_path.strip_prefix("data/").unwrap_or(bin_path);
    let stem = no_data.strip_suffix(".bin")?;
    Some(stem.to_owned())
}

fn remap_prop_bytes(
    raw: &[u8],
    donor_path: &str,
    target_path: &str,
) -> Result<Vec<u8>, ClassicError> {
    let mut prop = parse_prop_file(raw)?;
    let target_key = prop_record_key_from_bin_path(target_path)
        .map(|k| prop_key_hash(&k))
        .unwrap_or(0);
    let donor_key = prop_record_key_from_bin_path(donor_path)
        .map(|k| prop_key_hash(&k))
        .unwrap_or(0);
    if target_key != 0 && donor_key != 0 && target_key != donor_key {
        for entry in &mut prop.entries {
            if entry.key_hash == target_key {
                entry.key_hash = donor_key;
            }
        }
    }
    Ok(serialize_prop_file(&prop)?)
}

fn wad_path_for(game_dir: &Path, category: TftCategory) -> PathBuf {
    let final_dir = game_dir.join("DATA").join("FINAL");
    match category {
        TftCategory::Tactician => final_dir.join("Companions.wad.client"),
        TftCategory::Boom | TftCategory::Portal => final_dir.join("Global.wad.client"),
        TftCategory::Arena => final_dir
            .join("Maps")
            .join("Shipping")
            .join("Map22.wad.client"),
    }
}

fn apply_swap(
    game_dir: &Path,
    partial_dir: &Path,
    category: TftCategory,
    swap: &TftSwap,
) -> Result<bool, ClassicError> {
    let Some(donor) = normalize_asset_path(&swap.donor_path) else {
        warn!(donor = %swap.donor_path, ?category, "Invalid TFT donor path; skipped");
        return Ok(false);
    };
    let Some(target) = normalize_asset_path(&swap.target_path) else {
        warn!(target = %swap.target_path, ?category, "Invalid TFT target path; skipped");
        return Ok(false);
    };
    if donor == target {
        return Ok(false);
    }

    let wad_file_path = wad_path_for(game_dir, category);
    if !wad_file_path.is_file() {
        debug!(wad = %wad_file_path.display(), ?category, "TFT WAD archive not present");
        return Ok(false);
    }

    let wad = WadFile::open(&wad_file_path)?;
    let hash = wad_path_hash(&target);
    let Some(raw) = wad.read(hash)? else {
        warn!(target = %target, wad = %wad_file_path.display(), "TFT target entry missing in WAD");
        return Ok(false);
    };

    let output = if target.ends_with(".bin") && donor.ends_with(".bin") {
        remap_prop_bytes(&raw, &donor, &target)?
    } else {
        raw
    };

    let dst = partial_dir.join("RAW").join(&donor);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&dst, output)?;
    Ok(true)
}

fn remove_dir_quiet(path: &Path) {
    if path.exists() {
        let _ = std::fs::remove_dir_all(path); // ignore-ok: cleanup of previous TFT staging dir
    }
}

pub fn build_tft_mod(
    game_dir: &Path,
    mods_dir: &Path,
    loadout: &TftLoadout,
) -> Result<Option<String>, ClassicError> {
    let final_dir = mods_dir.join(TFT_MOD_FOLDER);
    if loadout.is_empty() {
        remove_dir_quiet(&final_dir);
        return Ok(None);
    }

    let partial = mods_dir.join(format!("{TFT_MOD_FOLDER}.partial"));
    remove_dir_quiet(&partial);
    std::fs::create_dir_all(partial.join("META"))?;

    let mut applied = Vec::new();
    let pairs = [
        (TftCategory::Tactician, loadout.tactician.as_ref()),
        (TftCategory::Boom, loadout.boom.as_ref()),
        (TftCategory::Portal, loadout.portal.as_ref()),
        (TftCategory::Arena, loadout.arena.as_ref()),
    ];

    for (category, maybe_swap) in pairs {
        if let Some(swap) = maybe_swap
            && apply_swap(game_dir, &partial, category, swap)?
        {
            applied.push(swap.name.clone());
        }
    }

    if applied.is_empty() {
        remove_dir_quiet(&partial);
        remove_dir_quiet(&final_dir);
        return Ok(None);
    }

    let info = serde_json::json!({
        "Author": "Dekan",
        "Description": applied.join(", "),
        "Name": "Dekan TFT Cosmetics",
        "Version": "1.0.0"
    });
    let info_bytes = serde_json::to_vec(&info)
        .map_err(|e| ClassicError::Io(std::io::Error::other(e.to_string())))?;
    std::fs::write(partial.join("META").join("info.json"), info_bytes)?;

    remove_dir_quiet(&final_dir);
    std::fs::rename(&partial, &final_dir)?;
    info!(items = ?applied, "TFT cosmetics mod built");
    Ok(Some(TFT_MOD_FOLDER.to_owned()))
}

#[cfg(test)]
mod tests;
