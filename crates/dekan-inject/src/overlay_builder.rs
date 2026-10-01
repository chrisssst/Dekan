use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dekan_wad::hash::{content_checksum, mount_name, relative_path_hash, wad_path_hash};
use dekan_wad::wad::{CompressionType, WadFile};
use dekan_wad::writer::{WadWriter, WriteOutcome, WriterEntry, optimal_raw, optimal_stored};
use tracing::{debug, info, warn};

use crate::error::InjectError;

const TFT_MOUNTS: [&str; 2] = ["map21", "map22"];

pub const OVERLAY_BUILDER_REVISION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeBuild {
    pub wad_files: usize,

    pub written: usize,

    pub bytes: u64,

    pub removed: usize,
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

struct CachedIndex {
    fingerprint: u64,
    index: Arc<GameIndexMap>,
}

static GAME_INDEX_CACHE: Mutex<BTreeMap<PathBuf, CachedIndex>> = Mutex::new(BTreeMap::new());

static PREWARM_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn get_or_index_game(game_dir: &Path) -> Result<Arc<GameIndexMap>, InjectError> {
    let mut files = Vec::new();
    collect_game_wads(&game_dir.join("DATA").join("FINAL"), &mut files);
    files.sort();
    let fingerprint = files_fingerprint(&files);
    let key = game_dir
        .canonicalize()
        .unwrap_or_else(|_| game_dir.to_path_buf());

    let mut cache = GAME_INDEX_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(cached) = cache.get(&key) {
        if cached.fingerprint == fingerprint {
            return Ok(Arc::clone(&cached.index));
        }
        info!(game = %game_dir.display(), "Game WAD files changed since they were indexed; indexing again");
    }
    let index = Arc::new(index_game(game_dir, files)?);
    cache.insert(
        key,
        CachedIndex {
            fingerprint,
            index: Arc::clone(&index),
        },
    );
    Ok(index)
}

fn files_fingerprint(files: &[PathBuf]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for file in files {
        file.hash(&mut hasher);
        match std::fs::metadata(file) {
            Ok(meta) => {
                meta.len().hash(&mut hasher);
                meta.modified().ok().hash(&mut hasher);
            }

            Err(_) => u64::MAX.hash(&mut hasher),
        }
    }
    hasher.finish()
}

pub fn prewarm_game_index(game_dir: &Path) {
    if PREWARM_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let dir = game_dir.to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("dekan-index-prewarm".into())
        .spawn(move || {
            let started = Instant::now();
            match get_or_index_game(&dir) {
                Ok(index) => debug!(
                    wads = index.len(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "Game WAD index ready"
                ),
                Err(e) => {
                    debug!(error = %e, "Game WAD index not prewarmed; the build will index on demand")
                }
            }
            PREWARM_RUNNING.store(false, Ordering::Release);
        });
    if let Err(e) = spawned {
        PREWARM_RUNNING.store(false, Ordering::Release);
        debug!(error = %e, "Game WAD index prewarm thread not started; the build will index on demand");
    }
}

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
    for name in mods {
        let mut index = index_mod(&mods_dir.join(name), name)?;
        for mount in index.mounts.values_mut() {
            mount.entries.retain(|hash, _| !blocked.contains(hash));
        }
        index.mounts.retain(|_, mount| !mount.entries.is_empty());
        if index.mounts.is_empty() {
            warn!(mod_name = %name, "Mod has nothing to merge; skipped");
            continue;
        }
        resolve_inside(&mut index);
        for older in &mut queue {
            resolve_against(older, &index);
        }
        queue.push(index);
        if cancelled() {
            return Err(stop());
        }
    }

    let mut overlay: BTreeMap<String, OverlayWad> = BTreeMap::new();
    let mut mounts = MountRoles::default();
    for index in &queue {
        add_overlay_mod(&game, index, &mut overlay, &mut mounts)?;
        if cancelled() {
            return Err(stop());
        }
    }
    log_shared_copies(&game, &mounts, mods);
    if mounts.identical > 0 {
        debug!(
            mods = ?mods,
            identical = mounts.identical,
            "Mod entries identical to the game were dropped as no-ops (H3, #165)"
        );
    }
    if mounts.withheld > 0 {
        warn!(
            mods = ?mods,
            withheld = mounts.withheld,
            "Mod entries that a map WAD also holds were left as the game has them; those assets keep their original look"
        );
    }

    let (mut written, mut bytes) = (0usize, 0u64);
    for (name, wad) in &overlay {
        let out = overlay_dir.join(&wad.relpath);
        let outcome = wad
            .writer
            .write_to_file(&out, &cancelled)
            .map_err(|e| match e {
                dekan_wad::error::WadError::Cancelled => stop(),
                other => {
                    InjectError::Overlay(format!("could not write '{}': {other}", out.display()))
                }
            })?;
        if matches!(outcome, WriteOutcome::Written { .. }) {
            written += 1;
        }
        bytes += outcome.bytes();
        debug!(mount = %name, entries = wad.writer.len(), outcome = ?outcome, "Overlay WAD ready");
    }

    let keep: HashSet<&str> = overlay.keys().map(String::as_str).collect();
    let removed = remove_strays(overlay_dir, &keep);

    let build = NativeBuild {
        wad_files: overlay.len(),
        written,
        bytes,
        removed,
        elapsed: started.elapsed(),
    };
    info!(
        mods = ?mods,
        wad_files = build.wad_files,
        written = build.written,
        overlay_bytes = build.bytes,
        removed = build.removed,
        game_index_ms = indexed_ms,
        elapsed_ms = build.elapsed.as_millis(),
        "Native overlay built"
    );
    Ok(build)
}

fn index_game(game_dir: &Path, files: Vec<PathBuf>) -> Result<GameIndexMap, InjectError> {
    let mut index = BTreeMap::new();
    let mut skipped = 0usize;
    for path in files {
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let wad = match WadFile::open_toc_only(&path) {
            Ok(wad) if wad.minor() == 4 => wad,
            Ok(wad) => {
                debug!(path = %path.display(), minor = wad.minor(), "Game WAD is not version 3.4; left out");
                skipped += 1;
                continue;
            }
            Err(e) => {
                debug!(path = %path.display(), error = %e, "Game WAD unreadable; left out");
                skipped += 1;
                continue;
            }
        };
        let mut names: Vec<u64> = wad.toc().map(|e| e.path_hash).collect();
        names.sort_unstable();
        let relpath = path.strip_prefix(game_dir).unwrap_or(&path).to_path_buf();
        index.insert(
            mount_name(file_name),
            GameWad {
                relpath,
                path: path.clone(),
                names,
            },
        );
    }
    for tft in TFT_MOUNTS {
        index.remove(tft);
    }
    if skipped > 0 {
        warn!(
            skipped,
            "Game WADs left out of the overlay index (unreadable or not v3.4)"
        );
    }
    Ok(index)
}

fn collect_game_wads(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {
                if !(name.ends_with(".wad") || name.ends_with(".wad.client")) {
                    collect_game_wads(&path, out);
                }
            }
            Ok(kind) if kind.is_file() && name.ends_with(".wad.client") => out.push(path),
            _ => {}
        }
    }
}

fn subchunk_toc_hash(relpath: &Path) -> u64 {
    wad_path_hash(
        &relpath
            .with_extension("SubChunkTOC")
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

fn index_mod(mod_dir: &Path, name: &str) -> Result<ModIndex, InjectError> {
    if !mod_dir.join("META").join("info.json").is_file() {
        return Err(InjectError::Overlay(format!(
            "not a valid mod (no META/info.json): {}",
            mod_dir.display()
        )));
    }
    let bad = |what: &str, path: &Path, e: &dyn std::fmt::Display| {
        InjectError::Overlay(format!("mod '{name}': {what} '{}': {e}", path.display()))
    };

    let mut mounts = BTreeMap::new();
    let wads = mod_dir.join("WAD");
    let mut children: Vec<PathBuf> = std::fs::read_dir(&wads)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    children.sort();
    for path in children {
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let entries = if path.is_file() {
            if !file_name.ends_with(".wad.client") {
                warn!(mod_name = name, file = %path.display(), "Not a .wad.client file; ignored");
                continue;
            }
            read_mod_wad(&path).map_err(|e| bad("unreadable WAD", &path, &e))?
        } else if path.is_dir() {
            if !(file_name.ends_with(".wad.client") || file_name.ends_with(".wad")) {
                warn!(mod_name = name, folder = %path.display(), "Not a .wad folder; ignored");
                continue;
            }
            pack_folder(&path).map_err(|e| bad("unreadable folder", &path, &e))?
        } else {
            continue;
        };
        mounts.insert(mount_name(file_name), ModMount { entries });
    }

    let raw = mod_dir.join("RAW");
    if raw.is_dir() {
        let entries = pack_folder(&raw).map_err(|e| bad("unreadable folder", &raw, &e))?;
        mounts.insert(mount_name("_RAW.wad.client"), ModMount { entries });
    }

    Ok(ModIndex {
        name: name.to_owned(),
        mounts,
    })
}

fn read_mod_wad(path: &Path) -> Result<BTreeMap<u64, WriterEntry>, dekan_wad::error::WadError> {
    let wad = WadFile::open(path)?;
    let mut entries = BTreeMap::new();
    for entry in wad.toc() {
        let stored = wad.read_raw(entry)?;
        let hash = entry.path_hash;
        let converted = optimal_stored(entry, stored, || {
            wad.read(hash)?.ok_or(dekan_wad::error::WadError::Internal(
                "an entry vanished from its own table of contents",
            ))
        })?;
        entries.insert(hash, converted);
    }
    Ok(entries)
}

fn pack_folder(dir: &Path) -> Result<BTreeMap<u64, WriterEntry>, dekan_wad::error::WadError> {
    let mut files = Vec::new();
    collect_files(dir, &mut files);
    let mut entries = BTreeMap::new();
    for file in files {
        let relative = file
            .strip_prefix(dir)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(&file).map_err(|e| dekan_wad::error::WadError::FileIo {
            path: file.display().to_string(),
            source: e,
        })?;
        entries.insert(relative_path_hash(&relative), optimal_raw(bytes)?);
    }
    Ok(entries)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = entries.flatten().collect();
    children.sort_by_key(|e| e.path());
    for entry in children {
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect_files(&entry.path(), out),
            Ok(kind) if kind.is_file() => out.push(entry.path()),
            _ => {}
        }
    }
}

fn resolve_inside(index: &mut ModIndex) {
    let names: Vec<String> = index.mounts.keys().cloned().collect();
    for a in &names {
        for b in &names {
            if a == b {
                continue;
            }
            let Some(source) = index.mounts.get(b).map(|m| m.entries.clone()) else {
                continue;
            };
            if let Some(target) = index.mounts.get_mut(a) {
                overwrite_shared(&mut target.entries, &source, &index.name);
            }
        }
    }
}

fn resolve_against(older: &mut ModIndex, newer: &ModIndex) {
    for mount in older.mounts.values_mut() {
        for other in newer.mounts.values() {
            overwrite_shared(&mut mount.entries, &other.entries, &newer.name);
        }
    }
}

fn overwrite_shared(
    target: &mut BTreeMap<u64, WriterEntry>,
    source: &BTreeMap<u64, WriterEntry>,
    winner: &str,
) {
    for (hash, entry) in target.iter_mut() {
        if let Some(newer) = source.get(hash) {
            if newer.checksum != entry.checksum {
                debug!(
                    path_hash = format_args!("{hash:#018x}"),
                    winner, "Conflicting mod entry; the newer one wins"
                );
            }
            *entry = newer.clone();
        }
    }
}

#[derive(Debug, Default)]
struct MountRoles {
    bases: HashSet<String>,
    shared: HashSet<String>,

    withheld: usize,

    identical: usize,
}

impl MountRoles {
    fn shared_only(&self) -> impl Iterator<Item = &String> {
        self.shared.difference(&self.bases)
    }
}

fn log_shared_copies(game: &BTreeMap<String, GameWad>, mounts: &MountRoles, mods: &[String]) {
    let (mut count, mut bytes, mut size_unknown) = (0usize, 0u64, 0usize);
    for name in mounts.shared_only() {
        count += 1;
        match game.get(name).map(|wad| std::fs::metadata(&wad.path)) {
            Some(Ok(meta)) => bytes += meta.len(),
            _ => size_unknown += 1,
        }
    }
    if count == 0 {
        return;
    }
    info!(
        mods = ?mods,
        shared_wads = count,
        shared_bytes = bytes,
        size_unknown,
        "Copying additional game WADs whole because they share entries with the mods"
    );
}

fn add_overlay_mod(
    game: &BTreeMap<String, GameWad>,
    index: &ModIndex,
    overlay: &mut BTreeMap<String, OverlayWad>,
    mounts: &mut MountRoles,
) -> Result<(), InjectError> {
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

        let entries = mergeable_entries(game, &base_name, &effective);
        mounts.withheld += effective.len() - entries.len();
        if entries.is_empty() {
            debug!(
                mod_name = %index.name,
                mount = %mount,
                "Nothing of this mount is merged (identical to the game, or held by a map WAD)"
            );
            continue;
        }

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
    Ok(())
}

fn decoded_mod_entry(entry: &WriterEntry) -> Option<Vec<u8>> {
    let bytes = match &entry.payload {
        dekan_wad::writer::Payload::Memory(bytes) => bytes,

        dekan_wad::writer::Payload::File { .. } => return None,
    };
    match CompressionType::from_type_byte(entry.kind) {
        Ok(CompressionType::Raw | CompressionType::Redirection) => Some(bytes.to_vec()),
        Ok(CompressionType::Zstd) => {
            zstd::bulk::decompress(bytes, entry.uncompressed_size as usize).ok()
        }
        _ => None,
    }
}

fn drop_entries_identical_to_game(
    game: &BTreeMap<String, GameWad>,
    base_name: &str,
    entries: &BTreeMap<u64, WriterEntry>,
) -> BTreeMap<u64, WriterEntry> {
    let Some(base) = game.get(base_name) else {
        return entries.clone();
    };
    let Ok(base_wad) = WadFile::open(&base.path) else {
        return entries.clone();
    };
    entries
        .iter()
        .filter(|(hash, entry)| {
            let identical = base
                .contains(**hash)
                .then(|| base_wad.read(**hash).ok().flatten())
                .flatten()
                .zip(decoded_mod_entry(entry))
                .is_some_and(|(game_bytes, mod_bytes)| {
                    content_checksum(&game_bytes) == content_checksum(&mod_bytes)
                });
            !identical
        })
        .map(|(hash, entry)| (*hash, entry.clone()))
        .collect()
}

fn is_map(wad: &GameWad) -> bool {
    wad.relpath
        .components()
        .nth(2)
        .is_some_and(|c| c.as_os_str().eq_ignore_ascii_case("Maps"))
}

fn mergeable_entries(
    game: &BTreeMap<String, GameWad>,
    base: &str,
    entries: &BTreeMap<u64, WriterEntry>,
) -> BTreeMap<u64, WriterEntry> {
    if game.get(base).is_some_and(is_map) {
        return entries.clone();
    }
    let maps: Vec<&GameWad> = game.values().filter(|wad| is_map(wad)).collect();
    entries
        .iter()
        .filter(|(hash, _)| !maps.iter().any(|map| map.contains(**hash)))
        .map(|(hash, entry)| (*hash, entry.clone()))
        .collect()
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
        let mut writer = WadWriter::new(*wad.signature());
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

/// Remove `.wad.client` files of earlier builds that this overlay no longer has, and partial files
/// a crash left behind (including the partials this writer creates).
fn remove_strays(dir: &Path, keep: &HashSet<&str>) -> usize {
    let mut removed = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind) if kind.is_file() => {
                    let stray = name.ends_with(".wad.client.partial")
                        || (name.ends_with(".wad.client")
                            && !keep.contains(mount_name(&name).as_str()));
                    if stray {
                        match std::fs::remove_file(&path) {
                            Ok(()) => removed += 1,
                            Err(e) => {
                                warn!(file = %path.display(), error = %e, "Stray overlay WAD could not be removed")
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    removed
}

#[cfg(test)]
#[path = "overlay_builder_tests.rs"]
mod tests;
