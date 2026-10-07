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

static INDEX_CACHE_FILE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

const INDEX_CACHE_MAGIC: &[u8; 8] = b"BIDX0001";

pub fn persist_game_index_in(dir: &Path) {
    let _ = INDEX_CACHE_FILE.set(dir.join("game_index.bin")); // ignore-ok: the first caller decides where the index lives for the whole run
}

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
    let index = match INDEX_CACHE_FILE.get() {
        Some(file) => match load_index(file, fingerprint, game_dir) {
            Some(index) => Arc::new(index),
            None => {
                let index = index_game(game_dir, files)?;
                store_index(file, fingerprint, &index);
                Arc::new(index)
            }
        },
        None => Arc::new(index_game(game_dir, files)?),
    };
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
    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    for file in files {
        hasher.update(file.to_string_lossy().as_bytes());
        let (len, modified) = std::fs::metadata(file).map_or((u64::MAX, 0), |meta| {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            (meta.len(), modified)
        });
        hasher.update(&len.to_le_bytes());
        hasher.update(&modified.to_le_bytes());
    }
    hasher.digest()
}

fn store_index(file: &Path, fingerprint: u64, index: &GameIndexMap) {
    let mut out = INDEX_CACHE_MAGIC.to_vec();
    out.extend_from_slice(&fingerprint.to_le_bytes());
    out.extend_from_slice(&(index.len() as u64).to_le_bytes());
    for (mount, wad) in index {
        let relpath = wad.relpath.to_string_lossy();
        for text in [mount.as_str(), relpath.as_ref()] {
            out.extend_from_slice(&(text.len() as u64).to_le_bytes());
            out.extend_from_slice(text.as_bytes());
        }
        out.extend_from_slice(&(wad.names.len() as u64).to_le_bytes());
        for name in &wad.names {
            out.extend_from_slice(&name.to_le_bytes());
        }
    }
    match dekan_platform::fs::atomic_write(file, &out, false) {
        Ok(()) => {
            debug!(file = %file.display(), bytes = out.len(), "Game WAD index saved for the next start")
        }
        Err(e) => {
            debug!(file = %file.display(), error = %e, "Game WAD index not saved; the next start indexes again")
        }
    }
}

fn load_index(file: &Path, fingerprint: u64, game_dir: &Path) -> Option<GameIndexMap> {
    let bytes = std::fs::read(file).ok()?;
    let index = parse_index(&bytes, fingerprint, game_dir);
    if index.is_none() {
        debug!(file = %file.display(), "Saved game WAD index is stale or unreadable; indexing again");
    }
    index
}

struct IndexReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> IndexReader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let slice = self.bytes.get(self.at..self.at.checked_add(len)?)?;
        self.at += len;
        Some(slice)
    }

    fn number(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn count(&mut self) -> Option<usize> {
        usize::try_from(self.number()?).ok()
    }

    fn text(&mut self) -> Option<String> {
        let len = self.count()?;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}

fn parse_index(bytes: &[u8], fingerprint: u64, game_dir: &Path) -> Option<GameIndexMap> {
    let mut reader = IndexReader { bytes, at: 0 };
    if reader.take(8)? != INDEX_CACHE_MAGIC || reader.number()? != fingerprint {
        return None;
    }
    let mut index = BTreeMap::new();
    for _ in 0..reader.count()? {
        let mount = reader.text()?;
        let relpath = PathBuf::from(reader.text()?);
        let count = reader.count()?;
        let names = reader
            .take(count.checked_mul(8)?)?
            .chunks_exact(8)
            .map(|chunk| chunk.try_into().ok().map(u64::from_le_bytes))
            .collect::<Option<Vec<u64>>>()?;
        let path = game_dir.join(&relpath);
        index.insert(
            mount,
            GameWad {
                relpath,
                path,
                names,
            },
        );
    }
    (reader.at == bytes.len()).then_some(index)
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

    for index in &mut queue {
        retype_stale_bins(&game, index)?;
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

    let base_store = base_store_for(overlay_dir);
    let revision = OVERLAY_BUILDER_REVISION.to_string();
    BUILDS_WAITING_FOR_COPIES.fetch_add(1, Ordering::AcqRel);
    let _copies = GAME_COPY_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    BUILDS_WAITING_FOR_COPIES.fetch_sub(1, Ordering::AcqRel);
    let (mut written, mut bytes) = (0usize, 0u64);
    let mut manifest = Vec::with_capacity(overlay.len());
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
        manifest.push(log_overlay_wad(
            game.get(name),
            name,
            wad,
            outcome,
            mode,
            &out,
        ));
    }
    write_overlay_manifest(overlay_dir, mods, &manifest);

    let keep: HashSet<&str> = overlay.keys().map(String::as_str).collect();
    let removed = remove_strays(overlay_dir, &keep, base_store.as_deref());

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
    let mut wads = Vec::new();
    collect_mod_wads(&mod_dir.join("WAD"), name, &mut wads);
    for path in wads {
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let entries = if path.is_file() {
            read_mod_wad(&path).map_err(|e| bad("unreadable WAD", &path, &e))?
        } else {
            pack_folder(&path).map_err(|e| bad("unreadable folder", &path, &e))?
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

fn collect_mod_wads(dir: &Path, mod_name: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    for path in children {
        let lower = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if lower.ends_with(".wad.client") || lower.ends_with(".wad") {
            out.push(path);
        } else if path.is_dir() {
            collect_mod_wads(&path, mod_name, out);
        } else {
            warn!(mod_name, file = %path.display(), "Not a WAD; ignored");
        }
    }
}

fn retype_stale_bins(game: &GameIndexMap, index: &mut ModIndex) -> Result<(), InjectError> {
    for (mount, mod_mount) in &mut index.mounts {
        let bins: Vec<(u64, Vec<u8>)> = mod_mount
            .entries
            .iter()
            .filter_map(|(hash, entry)| prop_payload(entry).map(|bytes| (*hash, bytes)))
            .collect();
        let Some(game_wad) = game.get(mount) else {
            continue;
        };
        if bins.is_empty() {
            continue;
        }
        let shapes = game_field_shapes(game_wad, mount, bins.iter().map(|(hash, _)| *hash))
            .map_err(|e| {
                InjectError::Overlay(format!(
                    "could not read the game's property types from '{}': {e}",
                    game_wad.path.display()
                ))
            })?;
        let mut retyped = 0;
        for (hash, bytes) in bins {
            match strings_to_files(&bytes, &shapes) {
                Ok(Some((fixed, count))) => {
                    let entry = optimal_raw(fixed).map_err(|e| {
                        InjectError::Overlay(format!("could not repack a retyped bin: {e}"))
                    })?;
                    mod_mount.entries.insert(hash, entry);
                    retyped += count;
                }
                Ok(None) => {}
                Err(e) => {
                    debug!(mod_name = %index.name, mount = %mount, path_hash = format!("{hash:016x}"), error = %e, "Mod bin not checked against the game's property types")
                }
            }
        }
        if retyped > 0 {
            info!(
                mod_name = %index.name,
                mount = %mount,
                properties = retyped,
                "Mod properties written as text were converted to the file references the game now expects"
            );
        }
    }
    Ok(())
}

fn game_field_shapes(
    game_wad: &GameWad,
    mount: &str,
    mod_bins: impl Iterator<Item = u64>,
) -> Result<FieldShapes, dekan_wad::error::WadError> {
    let wad = WadFile::open(&game_wad.path)?;
    let mut shapes = FieldShapes::default();
    let mut pending: Vec<u64> = mod_bins
        .chain([wad_path_hash(&format!(
            "data/characters/{mount}/animations/skin0.bin"
        ))])
        .collect();
    let mut seen = HashSet::new();
    while let Some(hash) = pending.pop() {
        if !seen.insert(hash) {
            continue;
        }
        let Some(data) = wad.read(hash)? else {
            continue;
        };
        if !is_prop(&data) {
            continue;
        }
        record_field_shapes(&data, &mut shapes)?;
        pending.extend(
            parse_prop_links(&data)?
                .iter()
                .map(|link| wad_path_hash(link)),
        );
    }
    Ok(shapes)
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

        let entries = effective;
        if entries.is_empty() {
            debug!(
                mod_name = %index.name,
                mount = %mount,
                "Nothing of this mount is merged (identical to the game)"
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
            dekan_wad::writer::decode_zstd_bounded(bytes, entry.uncompressed_size)
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
            let same_size = base.contains(**hash)
                && base_wad
                    .entry(**hash)
                    .is_some_and(|game| game.uncompressed_size as u64 == entry.uncompressed_size);
            let identical = same_size
                && base_wad
                    .read(**hash)
                    .ok()
                    .flatten()
                    .is_some_and(|game_bytes| {
                        decoded_mod_entry(entry).is_some_and(|mod_bytes| mod_bytes == game_bytes)
                    });
            !identical
        })
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

fn header_hex(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut head = [0u8; 272];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    Some(head.iter().map(|b| format!("{b:02x}")).collect())
}

fn log_overlay_wad(
    game_wad: Option<&GameWad>,
    mount: &str,
    wad: &OverlayWad,
    outcome: WriteOutcome,
    mode: &str,
    out: &Path,
) -> serde_json::Value {
    let game_toc = game_wad.and_then(|g| WadFile::open_toc_only(&g.path).ok());
    let mut entries = Vec::new();
    let (mut replaced, mut added) = (0usize, 0usize);
    for (hash, entry) in wad.writer.inserted() {
        let original = game_toc.as_ref().and_then(|t| t.entry(hash));
        if original.is_some() {
            replaced += 1;
        } else {
            added += 1;
        }
        let record = serde_json::json!({
            "path_hash": format!("{hash:016x}"),
            "change": if original.is_some() { "replaced" } else { "added" },
            "stored_bytes": WadWriter::stored_len_of(entry),
            "decoded_bytes": entry.uncompressed_size,
            "kind": entry.kind,
            "checksum": format!("{:016x}", entry.checksum),
            "game_stored_bytes": original.map(|o| o.compressed_size),
            "game_decoded_bytes": original.map(|o| o.uncompressed_size),
            "game_kind": original.map(|o| o.compression as u8),
            "game_checksum": original.map(|o| format!("{:016x}", o.checksum)),
        });
        debug!(mount, entry = %record, "Overlay entry");
        entries.push(record);
    }
    let game_header = game_wad.and_then(|g| header_hex(&g.path));
    let overlay_header = header_hex(out);
    let mode = if matches!(outcome, WriteOutcome::Unchanged { .. }) {
        "unchanged since last build"
    } else {
        mode
    };
    info!(
        mount,
        file = %wad.relpath.display(),
        map = game_wad.is_some_and(is_map),
        mode,
        entries = wad.writer.len(),
        replaced,
        added,
        header_matches_game = game_header.is_some() && game_header == overlay_header,
        bytes = outcome.bytes(),
        "Overlay WAD written"
    );
    serde_json::json!({
        "mount": mount,
        "file": wad.relpath.display().to_string(),
        "map": game_wad.is_some_and(is_map),
        "mode": mode,
        "entries": wad.writer.len(),
        "replaced": replaced,
        "added": added,
        "bytes": outcome.bytes(),
        "game_header": game_header,
        "overlay_header": overlay_header,
        "changes": entries,
    })
}

fn write_overlay_manifest(overlay_dir: &Path, mods: &[String], wads: &[serde_json::Value]) {
    let Some(parent) = overlay_dir.parent() else {
        return;
    };
    let manifest = serde_json::json!({
        "builder_revision": OVERLAY_BUILDER_REVISION,
        "mods": mods,
        "wads": wads,
    });
    let path = parent.join("overlay_manifest.json");
    match serde_json::to_vec_pretty(&manifest) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                warn!(file = %path.display(), error = %e, "Overlay manifest not written");
            }
        }
        Err(e) => warn!(error = %e, "Overlay manifest not serialized"),
    }
}

fn is_map(wad: &GameWad) -> bool {
    wad.relpath
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("Maps"))
}

#[must_use]
pub fn build_waiting_for_copies() -> bool {
    BUILDS_WAITING_FOR_COPIES.load(Ordering::Acquire) > 0
}

pub fn prewarm_shared_copies(
    game_dir: &Path,
    overlay_dir: &Path,
    names: &[u64],
    stop: &dyn Fn() -> bool,
) -> Result<usize, InjectError> {
    let Some(store) = base_store_for(overlay_dir) else {
        return Ok(0);
    };
    let game = get_or_index_game(game_dir)?;
    let revision = OVERLAY_BUILDER_REVISION.to_string();
    let _copies = GAME_COPY_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut copied = 0usize;
    for (mount, wad) in game.iter() {
        if !is_map(wad)
            || TFT_MOUNTS.contains(&mount.as_str())
            || !names.iter().any(|name| wad.contains(*name))
        {
            continue;
        }
        let served = overlay_dir.join(&wad.relpath);
        if served.is_file() && base_stamp_path(&served).is_file() {
            continue;
        }
        let started = Instant::now();
        let made = dekan_wad::writer::ensure_game_copy(
            &wad.path,
            &store.join(&wad.relpath),
            &revision,
            stop,
        )
        .map_err(|e| match e {
            dekan_wad::error::WadError::Cancelled => InjectError::Cancelled,
            other => {
                InjectError::Overlay(format!("could not copy '{}': {other}", wad.path.display()))
            }
        })?;
        if made {
            copied += 1;
            info!(
                mount = %mount,
                elapsed_ms = started.elapsed().as_millis(),
                "Map WAD copied ahead of the build; skins that share its paths build in milliseconds"
            );
        }
    }
    Ok(copied)
}

fn base_store_for(overlay_dir: &Path) -> Option<PathBuf> {
    overlay_dir
        .parent()
        .map(|parent| parent.join(BASE_STORE_DIR))
}

fn move_with_stamp(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(base_stamp_path(to)); // ignore-ok: an older stamp at the destination is replaced below
    std::fs::rename(from, to)?;
    std::fs::rename(base_stamp_path(from), base_stamp_path(to))
}

fn restore_base(stored: &Path, out: &Path) {
    if out.exists() || !stored.is_file() || !base_stamp_path(stored).is_file() {
        return;
    }
    if let Err(e) = move_with_stamp(stored, out) {
        debug!(file = %stored.display(), error = %e, "Kept game WAD copy not restored; it is copied again");
    }
}

fn remove_strays(dir: &Path, keep: &HashSet<&str>, base_store: Option<&Path>) -> usize {
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
                    let kept_copy = base_store.zip(path.strip_prefix(dir).ok()).filter(|_| {
                        name.ends_with(".wad.client") && base_stamp_path(&path).is_file()
                    });
                    if stray {
                        if let Some((store, relative)) = kept_copy {
                            match move_with_stamp(&path, &store.join(relative)) {
                                Ok(()) => {
                                    removed += 1;
                                    continue;
                                }
                                Err(e) => {
                                    debug!(file = %path.display(), error = %e, "Game WAD copy not kept; it is removed")
                                }
                            }
                        }
                        let _ = std::fs::remove_file(base_stamp_path(&path)); // ignore-ok: the stamp only describes the WAD removed below
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
