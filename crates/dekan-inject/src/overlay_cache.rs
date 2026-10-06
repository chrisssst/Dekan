use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dekan_wad::wad::WAD_HEADER_SIZE;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use xxhash_rust::xxh3::xxh3_64;

use crate::error::InjectError;
use crate::overlay_builder::OVERLAY_BUILDER_REVISION;

pub const FINGERPRINT_FILE: &str = ".overlay_fingerprint.json";

const CURRENT_SCHEMA_VERSION: u32 = 2;

fn builder_identity() -> (&'static str, u32) {
    ("native", OVERLAY_BUILDER_REVISION)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModFileRecord {
    pub rel_path: String,
    pub size: u64,
    pub mtime_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameWadRecord {
    pub rel_path: String,
    pub size: u64,
    pub header_checksum: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverlayWadRecord {
    pub rel_path: String,
    pub size: u64,
    pub header_checksum: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverlayFingerprint {
    pub version: u32,
    pub builder: String,
    pub builder_revision: u32,
    pub mods: Vec<String>,
    pub mod_files: BTreeMap<String, Vec<ModFileRecord>>,
    pub game_wads: Vec<GameWadRecord>,
    pub overlay_wads: Vec<OverlayWadRecord>,
    pub total_bytes: u64,
}

fn compute_header_checksum(path: &Path) -> Option<(u64, u64)> {
    let mut file = File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    let size = meta.len();
    if size < WAD_HEADER_SIZE as u64 {
        return None;
    }
    let mut buf = [0u8; WAD_HEADER_SIZE];
    file.read_exact(&mut buf).ok()?;
    let checksum = xxh3_64(&buf);
    Some((size, checksum))
}

fn scan_mod_files(mod_dir: &Path) -> Vec<ModFileRecord> {
    let mut records = Vec::new();
    fn walk(root: &Path, current: &Path, records: &mut Vec<ModFileRecord>) {
        let Ok(entries) = std::fs::read_dir(current) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                walk(root, &path, records);
            } else if ft.is_file() {
                let Ok(rel) = path.strip_prefix(root) else {
                    continue;
                };
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let size = meta.len();
                let mtime_secs = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                records.push(ModFileRecord {
                    rel_path: rel_str,
                    size,
                    mtime_secs,
                });
            }
        }
    }
    walk(mod_dir, mod_dir, &mut records);
    records.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    records
}

fn scan_wads(root: &Path) -> Vec<PathBuf> {
    let mut wads = Vec::new();
    fn walk(root: &Path, current: &Path, wads: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(current) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                walk(root, &path, wads);
            } else if ft.is_file() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.to_ascii_lowercase().ends_with(".wad.client") {
                    if let Ok(rel) = path.strip_prefix(root) {
                        wads.push(rel.to_path_buf());
                    }
                }
            }
        }
    }
    walk(root, root, &mut wads);
    wads.sort();
    wads
}

pub struct OverlayCache;

impl OverlayCache {
    #[must_use]
    pub fn is_fresh(
        game_dir: &Path,
        mods_dir: &Path,
        overlay_dir: &Path,
        mods: &[String],
    ) -> Option<(usize, u64)> {
        let fingerprint_path = overlay_dir.join(FINGERPRINT_FILE);
        let bytes = std::fs::read(&fingerprint_path).ok()?;
        let fp: OverlayFingerprint = serde_json::from_slice(&bytes).ok()?;

        if fp.version != CURRENT_SCHEMA_VERSION {
            debug!("Overlay cache: schema version mismatch");
            return None;
        }

        let (builder_name, builder_revision) = builder_identity();
        if fp.builder != builder_name || fp.builder_revision != builder_revision {
            debug!(
                recorded = %fp.builder,
                recorded_revision = fp.builder_revision,
                configured = builder_name,
                configured_revision = builder_revision,
                "Overlay cache: built by another builder or builder revision"
            );
            return None;
        }

        if fp.mods != mods {
            debug!(expected = ?fp.mods, requested = ?mods, "Overlay cache: mod list changed");
            return None;
        }

        for name in mods {
            let Some(expected_files) = fp.mod_files.get(name) else {
                debug!(mod_name = %name, "Overlay cache: mod not in fingerprint");
                return None;
            };
            let current_files = scan_mod_files(&mods_dir.join(name));
            if expected_files != &current_files {
                debug!(mod_name = %name, "Overlay cache: mod files modified or missing");
                return None;
            }
        }

        for gw in &fp.game_wads {
            let game_wad_path = game_dir.join(&gw.rel_path);
            let Some((size, checksum)) = compute_header_checksum(&game_wad_path) else {
                debug!(game_wad = %gw.rel_path, "Overlay cache: game WAD missing or unreadable");
                return None;
            };
            if size != gw.size || checksum != gw.header_checksum {
                debug!(
                    game_wad = %gw.rel_path,
                    "Overlay cache: game WAD changed (game patched)"
                );
                return None;
            }
        }

        if fp.overlay_wads.is_empty() {
            debug!("Overlay cache: fingerprint has 0 overlay WADs");
            return None;
        }

        let mut observed_bytes = 0u64;
        for ow in &fp.overlay_wads {
            let overlay_wad_path = overlay_dir.join(&ow.rel_path);
            let Some((size, checksum)) = compute_header_checksum(&overlay_wad_path) else {
                debug!(overlay_wad = %ow.rel_path, "Overlay cache: overlay WAD missing or corrupted");
                return None;
            };
            if size != ow.size || checksum != ow.header_checksum {
                debug!(overlay_wad = %ow.rel_path, "Overlay cache: overlay WAD content mismatch");
                return None;
            }
            observed_bytes += size;
        }

        let disk_wads = scan_wads(overlay_dir);
        if disk_wads.len() != fp.overlay_wads.len() {
            debug!(
                disk = disk_wads.len(),
                expected = fp.overlay_wads.len(),
                "Overlay cache: stray WADs detected on disk"
            );
            return None;
        }

        Some((fp.overlay_wads.len(), observed_bytes))
    }

    pub fn record(
        game_dir: &Path,
        mods_dir: &Path,
        overlay_dir: &Path,
        mods: &[String],
    ) -> Result<(), InjectError> {
        let mut mod_files = BTreeMap::new();
        for name in mods {
            let files = scan_mod_files(&mods_dir.join(name));
            mod_files.insert(name.clone(), files);
        }

        let overlay_wad_paths = scan_wads(overlay_dir);
        let mut overlay_wads = Vec::with_capacity(overlay_wad_paths.len());
        let mut game_wads = Vec::new();
        let mut total_bytes = 0u64;

        for rel in overlay_wad_paths {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let overlay_path = overlay_dir.join(&rel);
            let Some((size, checksum)) = compute_header_checksum(&overlay_path) else {
                return Err(InjectError::Overlay(format!(
                    "failed to read header for built overlay WAD: {}",
                    overlay_path.display()
                )));
            };
            total_bytes += size;
            overlay_wads.push(OverlayWadRecord {
                rel_path: rel_str.clone(),
                size,
                header_checksum: checksum,
            });

            let game_path = game_dir.join(&rel);
            if game_path.exists() {
                if let Some((g_size, g_checksum)) = compute_header_checksum(&game_path) {
                    game_wads.push(GameWadRecord {
                        rel_path: rel_str,
                        size: g_size,
                        header_checksum: g_checksum,
                    });
                }
            }
        }

        let (builder_name, builder_revision) = builder_identity();
        let fp = OverlayFingerprint {
            version: CURRENT_SCHEMA_VERSION,
            builder: builder_name.to_owned(),
            builder_revision,
            mods: mods.to_vec(),
            mod_files,
            game_wads,
            overlay_wads,
            total_bytes,
        };

        let encoded = serde_json::to_vec_pretty(&fp)
            .map_err(|e| InjectError::Overlay(format!("failed to serialize fingerprint: {e}")))?;

        let temp_path = overlay_dir.join(".overlay_fingerprint.tmp");
        std::fs::write(&temp_path, encoded).map_err(InjectError::Io)?;

        let final_path = overlay_dir.join(FINGERPRINT_FILE);
        std::fs::rename(&temp_path, &final_path).map_err(|e| {
            let _ = std::fs::remove_file(&temp_path); // ignore-ok: cleanup on rename error
            InjectError::Io(e)
        })?;

        info!(
            mods = ?mods,
            builder = builder_name,
            wads = fp.overlay_wads.len(),
            bytes = total_bytes,
            "Overlay cache fingerprint recorded"
        );
        Ok(())
    }

    pub fn invalidate(overlay_dir: &Path) {
        let path = overlay_dir.join(FINGERPRINT_FILE);
        let _ = std::fs::remove_file(path); // ignore-ok: absent file is already invalidated
    }
}

#[cfg(test)]
#[path = "overlay_cache_tests.rs"]
mod tests;
