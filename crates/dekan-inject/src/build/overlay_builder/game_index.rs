use super::*;

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

pub(super) fn files_fingerprint(files: &[PathBuf]) -> u64 {
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

pub(super) fn store_index(file: &Path, fingerprint: u64, index: &GameIndexMap) {
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

pub(super) fn load_index(file: &Path, fingerprint: u64, game_dir: &Path) -> Option<GameIndexMap> {
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

pub(super) fn parse_index(bytes: &[u8], fingerprint: u64, game_dir: &Path) -> Option<GameIndexMap> {
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

pub(super) fn index_game(
    game_dir: &Path,
    files: Vec<PathBuf>,
) -> Result<GameIndexMap, InjectError> {
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

pub(super) fn collect_game_wads(dir: &Path, out: &mut Vec<PathBuf>) {
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

pub(super) fn subchunk_toc_hash(relpath: &Path) -> u64 {
    wad_path_hash(
        &relpath
            .with_extension("SubChunkTOC")
            .to_string_lossy()
            .replace('\\', "/"),
    )
}
