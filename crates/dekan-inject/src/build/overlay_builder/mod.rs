use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dekan_wad::hash::{mount_name, relative_path_hash, wad_path_hash};
use dekan_wad::prop::tree::FieldShapes;
use dekan_wad::prop::{is_prop, parse_prop_links, record_field_shapes, strings_to_files};
use dekan_wad::wad::{CompressionType, WadFile};
use dekan_wad::writer::{
    WadWriter, WriteOutcome, WriterEntry, base_stamp_path, optimal_raw, optimal_stored,
    prop_payload,
};
use tracing::{debug, info, warn};

use crate::error::InjectError;

const TFT_MOUNTS: [&str; 2] = ["map21", "map22"];

const BASE_STORE_DIR: &str = "overlay_base";

static GAME_COPY_LOCK: Mutex<()> = Mutex::new(());
static BUILDS_WAITING_FOR_COPIES: AtomicUsize = AtomicUsize::new(0);

pub const OVERLAY_BUILDER_REVISION: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeBuild {
    pub wad_files: usize,

    pub written: usize,

    pub bytes: u64,

    pub removed: usize,

    pub stuck: usize,
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
pub struct GameWad {
    pub relpath: PathBuf,
    pub path: PathBuf,
    pub names: Vec<u64>,
}

impl GameWad {
    pub fn contains(&self, name: u64) -> bool {
        self.names.binary_search(&name).is_ok()
    }
}

type GameIndexMap = BTreeMap<String, GameWad>;

mod game_index;
mod mods;
mod report;
mod store;

use game_index::*;
pub use game_index::{get_or_index_game, persist_game_index_in, prewarm_game_index};
use mods::*;
use report::*;
use store::*;
pub use store::{build_waiting_for_copies, prewarm_shared_copies};

#[derive(Debug, Clone)]
struct ModMount {
    entries: BTreeMap<u64, WriterEntry>,
}

#[derive(Debug)]
struct ModIndex {
    name: String,
    mounts: BTreeMap<String, ModMount>,
}

struct OverlayWad {
    relpath: PathBuf,
    writer: WadWriter,
}

pub fn build(
    game_dir: &Path,
    mods_dir: &Path,
    overlay_dir: &Path,
    mods: &[String],
    cancel: &AtomicBool,
) -> Result<NativeBuild, InjectError> {
    let started = Instant::now();
    let cancelled = || cancel.load(Ordering::Relaxed);
    let stop = || InjectError::Cancelled;

    let game = get_or_index_game(game_dir)?;
    if game.is_empty() {
        return Err(InjectError::Overlay(format!(
            "not a valid game folder (no WAD under DATA/FINAL): {}",
            game_dir.display()
        )));
    }
    let blocked: HashSet<u64> = game
        .values()
        .map(|g| subchunk_toc_hash(&g.relpath))
        .collect();
    let indexed_ms = started.elapsed().as_millis();
    if cancelled() {
        return Err(stop());
    }

    let mut queue: Vec<ModIndex> = Vec::with_capacity(mods.len());
    let mut no_op: Vec<&str> = Vec::new();
    let (mut blocked_entries, mut conflicts) = (0usize, 0usize);
    for name in mods {
        let mut index = index_mod(&mods_dir.join(name), name)?;
        for mount in index.mounts.values_mut() {
            let before = mount.entries.len();
            mount.entries.retain(|hash, _| !blocked.contains(hash));
            blocked_entries += before - mount.entries.len();
        }
        index.mounts.retain(|_, mount| !mount.entries.is_empty());
        if index.mounts.is_empty() {
            no_op.push(name);
            continue;
        }
        conflicts += resolve_inside(&mut index);
        for older in &mut queue {
            conflicts += resolve_against(older, &index);
        }
        queue.push(index);
        if cancelled() {
            return Err(stop());
        }
    }

    for index in &mut queue {
        retype_stale_bins(&game, index)?;
    }

    let mut overlay: BTreeMap<String, OverlayWad> = BTreeMap::new();
    let mut mounts = MountRoles::default();
    for index in &queue {
        if !add_overlay_mod(&game, index, &mut overlay, &mut mounts)? {
            no_op.push(&index.name);
        }
        if cancelled() {
            return Err(stop());
        }
    }
    if !no_op.is_empty() {
        warn!(
            mods = ?no_op,
            "Mods with nothing that differs from the game were skipped; they change nothing this match"
        );
    }
    if blocked_entries > 0 {
        debug!(
            blocked_entries,
            "Mod entries that would replace a game SubChunkTOC were left out"
        );
    }
    if conflicts > 0 {
        info!(
            mods = ?mods,
            conflicts,
            "Mods replace the same entries; the mod later in the list wins"
        );
    }
    log_shared_copies(&game, &mounts, mods);
    if mounts.identical > 0 {
        debug!(
            mods = ?mods,
            identical = mounts.identical,
            "Mod entries identical to the game were dropped as no-ops (H3, #165)"
        );
    }

    let base_store = base_store_for(overlay_dir);
    let revision = OVERLAY_BUILDER_REVISION.to_string();
    BUILDS_WAITING_FOR_COPIES.fetch_add(1, Ordering::AcqRel);
    let _copies = GAME_COPY_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    BUILDS_WAITING_FOR_COPIES.fetch_sub(1, Ordering::AcqRel);
    let (mut written, mut bytes) = (0usize, 0u64);
    let mut manifest = Vec::with_capacity(overlay.len());
    let mut header_mismatch = 0usize;
    for (name, wad) in &overlay {
        let out = overlay_dir.join(&wad.relpath);
        let failed = |e: dekan_wad::error::WadError| match e {
            dekan_wad::error::WadError::Cancelled => stop(),
            other => InjectError::Overlay(format!("could not write '{}': {other}", out.display())),
        };
        if let Some(store) = &base_store {
            restore_base(&store.join(&wad.relpath), &out);
        }
        let (outcome, mode) = match wad
            .writer
            .write_over_game_copy(&out, &revision, &cancelled)
            .map_err(failed)?
        {
            Some(outcome) => (outcome, "game copy + appended entries"),
            None => (
                wad.writer.write_to_file(&out, &cancelled).map_err(failed)?,
                "full rewrite",
            ),
        };
        if matches!(outcome, WriteOutcome::Written { .. }) {
            written += 1;
        }
        bytes += outcome.bytes();
        let (record, header_matches) =
            log_overlay_wad(game.get(name), name, wad, outcome, mode, &out);
        header_mismatch += usize::from(!header_matches);
        manifest.push(record);
    }
    write_overlay_manifest(overlay_dir, mods, &manifest);

    let keep: HashSet<&str> = overlay.keys().map(String::as_str).collect();
    let strays = remove_strays(overlay_dir, &keep, base_store.as_deref());

    let build = NativeBuild {
        wad_files: overlay.len(),
        written,
        bytes,
        removed: strays.removed,
        stuck: strays.stuck + strays.unlisted,
        elapsed: started.elapsed(),
    };
    info!(
        mods = ?mods,
        wad_files = build.wad_files,
        written = build.written,
        overlay_bytes = build.bytes,
        removed = build.removed,
        stuck = build.stuck,
        header_mismatch,
        game_index_ms = indexed_ms,
        elapsed_ms = build.elapsed.as_millis(),
        "Native overlay built"
    );
    Ok(build)
}

#[derive(Debug, Default)]
struct MountRoles {
    bases: HashSet<String>,
    shared: HashSet<String>,

    identical: usize,
}

impl MountRoles {
    fn shared_only(&self) -> impl Iterator<Item = &String> {
        self.shared.difference(&self.bases)
    }
}

fn add_overlay_mod(
    game: &BTreeMap<String, GameWad>,
    index: &ModIndex,
    overlay: &mut BTreeMap<String, OverlayWad>,
    mounts: &mut MountRoles,
) -> Result<bool, InjectError> {
    let mut merged = false;
    for (mount, content) in &index.mounts {
        let base_name = match game.get(mount) {
            Some(_) => mount.clone(),
            None => find_by_overlap(game, &content.entries).ok_or_else(|| {
                InjectError::Overlay(format!(
                    "mod '{}': no game WAD for '{mount}' (no name match, no shared entry)",
                    index.name
                ))
            })?,
        };

        let effective = drop_entries_identical_to_game(game, &base_name, &content.entries);
        let dropped = content.entries.len() - effective.len();
        mounts.identical += dropped;
        if dropped > 0 {
            debug!(
                mod_name = %index.name,
                mount = %mount,
                dropped,
                "Mod entries identical to the game were dropped (no-ops, #165)"
            );
        }

        let entries = effective;
        if entries.is_empty() {
            debug!(
                mod_name = %index.name,
                mount = %mount,
                "Nothing of this mount is merged (identical to the game)"
            );
            continue;
        }

        merged = true;
        let base = clone_into(overlay, game, &base_name)?;
        for (hash, entry) in &entries {
            base.writer.insert(*hash, entry.clone());
        }
        mounts.bases.insert(base_name.clone());

        for (other_name, other) in game {
            if *other_name == base_name {
                continue;
            }
            let shared: Vec<(&u64, &WriterEntry)> = entries
                .iter()
                .filter(|(hash, _)| other.contains(**hash))
                .collect();
            if shared.is_empty() {
                continue;
            }
            debug!(
                mod_name = %index.name,
                mount = %other_name,
                shared = shared.len(),
                "Game WAD shares entries with the mod; copied into the overlay too"
            );
            let copy = clone_into(overlay, game, other_name)?;
            for (hash, entry) in shared {
                copy.writer.insert(*hash, entry.clone());
            }
            mounts.shared.insert(other_name.clone());
        }
    }
    Ok(merged)
}

fn find_by_overlap(
    game: &BTreeMap<String, GameWad>,
    entries: &BTreeMap<u64, WriterEntry>,
) -> Option<String> {
    let mut best: Option<(&String, usize)> = None;
    for (name, wad) in game {
        let count = entries.keys().filter(|hash| wad.contains(**hash)).count();
        if count > best.map_or(0, |(_, c)| c) {
            best = Some((name, count));
        }
    }
    best.map(|(name, _)| name.clone())
}

fn clone_into<'a>(
    overlay: &'a mut BTreeMap<String, OverlayWad>,
    game: &BTreeMap<String, GameWad>,
    name: &str,
) -> Result<&'a mut OverlayWad, InjectError> {
    if !overlay.contains_key(name) {
        let source = game
            .get(name)
            .ok_or_else(|| InjectError::Overlay(format!("game mount '{name}' vanished")))?;
        let wad = WadFile::open_toc_only(&source.path).map_err(|e| {
            InjectError::Overlay(format!(
                "could not read game WAD '{}': {e}",
                source.path.display()
            ))
        })?;
        let mut writer = WadWriter::rebased_on(&wad);
        let index = writer.add_source(&source.path);
        for entry in wad.toc() {
            writer.insert(entry.path_hash, WriterEntry::from_wad(index, entry));
        }
        overlay.insert(
            name.to_owned(),
            OverlayWad {
                relpath: source.relpath.clone(),
                writer,
            },
        );
    }
    overlay
        .get_mut(name)
        .ok_or_else(|| InjectError::Overlay(format!("overlay mount '{name}' vanished")))
}

fn is_map(wad: &GameWad) -> bool {
    wad.relpath
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("Maps"))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod store_tests;

#[cfg(test)]
mod mods_tests;
