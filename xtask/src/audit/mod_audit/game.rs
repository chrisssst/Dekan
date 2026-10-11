use super::*;

pub(super) struct Game {
    pub(super) index: std::sync::Arc<BTreeMap<String, GameWad>>,
    pub(super) hashes: HashSet<u64>,
    pub(super) opened: HashMap<PathBuf, Option<WadFile>>,
    pub(super) archive_keys: HashMap<PathBuf, HashSet<u32>>,
    pub(super) global_keys: Option<HashSet<u32>>,
}

impl Game {
    pub(super) fn read(&mut self, hash: u64) -> Option<Vec<u8>> {
        let holder = self.index.values().find(|wad| wad.contains(hash))?;
        self.opened
            .entry(holder.path.clone())
            .or_insert_with(|| WadFile::open(&holder.path).ok())
            .as_ref()?
            .read(hash)
            .ok()
            .flatten()
    }

    pub(super) fn global_keys(&mut self) -> HashSet<u32> {
        if let Some(keys) = &self.global_keys {
            return keys.clone();
        }
        let shared: Vec<PathBuf> = self
            .index
            .values()
            .filter(|wad| {
                let relative = wad
                    .relpath
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_ascii_lowercase();
                relative.ends_with("/data.wad.client")
                    || relative.ends_with("/global.wad.client")
                    || relative.contains("/shaders/")
            })
            .map(|wad| wad.path.clone())
            .collect();
        let started = std::time::Instant::now();
        let mut keys = HashSet::new();
        for path in &shared {
            keys.extend(self.prop_keys_of(path));
        }
        println!(
            "objetos globais do jogo: {} chaves em {} WADs ({} ms)",
            keys.len(),
            shared.len(),
            started.elapsed().as_millis()
        );
        self.global_keys = Some(keys.clone());
        keys
    }

    pub(super) fn prop_keys_of(&mut self, path: &Path) -> HashSet<u32> {
        if let Some(found) = self.archive_keys.get(path) {
            return found.clone();
        }
        let mut found = HashSet::new();
        if let Ok(wad) = WadFile::open(path) {
            let mut entries: Vec<(usize, u64)> =
                wad.toc().map(|e| (e.offset, e.path_hash)).collect();
            entries.sort_unstable();
            for (_, name) in entries {
                let is_bin = wad
                    .read_prefix(name, 4)
                    .ok()
                    .flatten()
                    .is_some_and(|head| head == b"PROP" || head == b"PTCH");
                if !is_bin {
                    continue;
                }
                if let Some(prop) = wad
                    .read(name)
                    .ok()
                    .flatten()
                    .and_then(|b| parse_prop_file(&b).ok())
                {
                    found.extend(prop.entries.iter().map(|e| e.key_hash));
                }
            }
        }
        self.archive_keys.insert(path.to_path_buf(), found.clone());
        found
    }

    pub(super) fn keys_of_archives_holding(&mut self, hashes: &BTreeSet<u64>) -> HashSet<u32> {
        let holders: BTreeSet<PathBuf> = self
            .index
            .values()
            .filter(|wad| hashes.iter().any(|h| wad.contains(*h)))
            .map(|wad| wad.path.clone())
            .collect();
        let mut keys = HashSet::new();
        for path in holders {
            let found = self.archive_keys.entry(path.clone()).or_insert_with(|| {
                let mut found = HashSet::new();
                if let Ok(wad) = WadFile::open(&path) {
                    let names: Vec<u64> = wad.toc().map(|e| e.path_hash).collect();
                    for name in names {
                        let is_bin = wad
                            .read_prefix(name, 4)
                            .ok()
                            .flatten()
                            .is_some_and(|head| head == b"PROP" || head == b"PTCH");
                        if !is_bin {
                            continue;
                        }
                        if let Some(prop) = wad
                            .read(name)
                            .ok()
                            .flatten()
                            .and_then(|b| parse_prop_file(&b).ok())
                        {
                            found.extend(prop.entries.iter().map(|e| e.key_hash));
                        }
                    }
                }
                found
            });
            keys.extend(found.iter());
        }
        keys
    }
}

pub(super) fn extract(archive: &Path, into: &Path) -> Result<(), String> {
    if into.exists() {
        std::fs::remove_dir_all(into).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    safe_extract_zip(std::io::BufReader::new(file), into, &LIMITS)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub(super) fn pack_folder(folder: &Path, target: &Path) -> Result<(), String> {
    let mut writer = WadWriter::default();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(folder)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            writer.insert(
                relative_path_hash(&relative),
                optimal_raw(bytes).map_err(|e| e.to_string())?,
            );
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    writer
        .write_to_file(target, &|| false)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub(super) struct ModWad {
    pub(super) source: PathBuf,
    pub(super) readable: PathBuf,
}

pub(super) fn wads(mod_dir: &Path, packed: &Path) -> Result<BTreeMap<String, ModWad>, String> {
    let mut out = BTreeMap::new();
    for source in mod_wads(mod_dir) {
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let readable = if source.is_dir() {
            let target = packed.join(&name);
            pack_folder(&source, &target)?;
            target
        } else {
            source.clone()
        };
        out.insert(name, ModWad { source, readable });
    }
    Ok(out)
}
