use super::*;

#[must_use]
pub fn jade_characters(hashes_path: &Path, cache_path: &Path) -> BTreeSet<String> {
    let meta = match std::fs::metadata(hashes_path) {
        Ok(meta) => meta,
        Err(e) => {
            warn!(
                table = %hashes_path.display(),
                error = %e,
                "Game hash table unavailable; Rift Classic companion characters will be recovered from the champion bins"
            );
            return BTreeSet::new();
        }
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let fingerprint = format!("{}:{mtime}", meta.len());

    if let Ok(bytes) = std::fs::read(cache_path) {
        if let Ok(cached) = serde_json::from_slice::<CharacterCache>(&bytes) {
            if cached.source == fingerprint {
                debug!(
                    characters = cached.characters.len(),
                    "Rift Classic character index from cache"
                );
                return cached.characters;
            }
        }
    }

    let characters = match scan_jade_characters(hashes_path) {
        Ok(characters) => characters,
        Err(e) => {
            warn!(
                table = %hashes_path.display(),
                error = %e,
                "Game hash table could not be read; Rift Classic companion characters will be recovered from the champion bins"
            );
            return BTreeSet::new();
        }
    };

    let cache = CharacterCache {
        source: fingerprint,
        characters: characters.clone(),
    };
    match serde_json::to_vec(&cache) {
        Ok(bytes) => {
            if let Some(parent) = cache_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    debug!(error = %e, "Rift Classic character cache folder unavailable");
                }
            }
            if let Err(e) = std::fs::write(cache_path, bytes) {
                debug!(error = %e, "Rift Classic character index could not be cached");
            }
        }
        Err(e) => debug!(error = %e, "Rift Classic character index could not be serialized"),
    }
    info!(
        characters = characters.len(),
        "Rift Classic characters indexed from the game hash table"
    );
    characters
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct CharacterCache {
    pub(crate) source: String,
    pub(crate) characters: BTreeSet<String>,
}

pub(crate) fn scan_jade_characters(hashes_path: &Path) -> std::io::Result<BTreeSet<String>> {
    use std::io::BufRead;

    const NEEDLE: &[u8] = b"data/characters/jade_";
    let file = std::fs::File::open(hashes_path)?;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    let mut line = Vec::new();
    let mut found = BTreeSet::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let Some(start) = line.windows(NEEDLE.len()).position(|w| w == NEEDLE) else {
            continue;
        };
        let name_start = start + b"data/characters/".len();
        let name: Vec<u8> = line[name_start..]
            .iter()
            .copied()
            .take_while(|b| *b != b'/')
            .collect();

        let terminated = line.get(name_start + name.len()) == Some(&b'/');
        if terminated
            && name
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
        {
            if let Ok(name) = String::from_utf8(name) {
                found.insert(name);
            }
        }
    }
    Ok(found)
}

pub(crate) fn wad_stamp(path: &Path) -> String {
    std::fs::metadata(path)
        .map(|meta| {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!("{}:{mtime}", meta.len())
        })
        .unwrap_or_default()
}

pub(crate) fn character_names_in_bins(wad: &WadFile, alias: &str) -> BTreeSet<String> {
    const MAX_BIN_BYTES: usize = 8 * 1024 * 1024;
    const NEEDLE: &[u8] = b"characters/";

    let mut found = BTreeSet::new();
    let mut unreadable = 0usize;
    let mut on_disk: Vec<&dekan_wad::WadEntry> = wad
        .toc()
        .filter(|entry| entry.uncompressed_size <= MAX_BIN_BYTES)
        .collect();
    on_disk.sort_unstable_by_key(|entry| entry.offset);
    for hash in on_disk.into_iter().map(|entry| entry.path_hash) {
        match wad.read_prefix(hash, 4) {
            Ok(Some(head)) if head.starts_with(b"PROP") || head.starts_with(b"PTCH") => {}
            Ok(_) => continue,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        }
        let bytes = match wad.read(hash) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        };
        if !(bytes.starts_with(b"PROP") || bytes.starts_with(b"PTCH")) {
            continue;
        }
        let lower = bytes.to_ascii_lowercase();
        let mut from = 0;
        while let Some(pos) = lower[from..]
            .windows(NEEDLE.len())
            .position(|w| w == NEEDLE)
        {
            let name_start = from + pos + NEEDLE.len();
            let name: Vec<u8> = lower[name_start..]
                .iter()
                .copied()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .collect();
            if !name.is_empty() && lower.get(name_start + name.len()) == Some(&b'/') {
                if let Ok(name) = String::from_utf8(name) {
                    found.insert(name);
                }
            }
            from = name_start;
        }
    }
    if unreadable > 0 {
        warn!(
            alias,
            unreadable,
            "Some entries of the champion WAD could not be read while looking for character names"
        );
    }
    found
}

pub(crate) fn cached_names(
    cache_path: &Path,
    stamp: &str,
    alias: &str,
    ahead_of_time: bool,
    scan: impl FnOnce() -> BTreeSet<String>,
) -> BTreeSet<String> {
    if !stamp.is_empty() {
        if let Ok(bytes) = std::fs::read(cache_path) {
            if let Ok(cached) = serde_json::from_slice::<CharacterCache>(&bytes) {
                if cached.source == stamp {
                    debug!(
                        alias,
                        characters = cached.characters.len(),
                        "Character names from the bin-scan cache"
                    );
                    return cached.characters;
                }
            }
        }
    }

    let started = std::time::Instant::now();
    let characters = scan();
    if ahead_of_time {
        debug!(
            alias,
            names = ?characters,
            elapsed_ms = started.elapsed().as_millis(),
            "Character names indexed ahead of champion select"
        );
    } else {
        info!(
            alias,
            names = ?characters,
            elapsed_ms = started.elapsed().as_millis(),
            "Character names recovered from the champion's bins"
        );
    }
    if !stamp.is_empty() {
        let cache = CharacterCache {
            source: stamp.to_owned(),
            characters: characters.clone(),
        };
        match serde_json::to_vec(&cache) {
            Ok(bytes) => {
                if let Some(parent) = cache_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        debug!(error = %e, "Bin-scan cache folder unavailable");
                    }
                }
                if let Err(e) = std::fs::write(cache_path, bytes) {
                    debug!(error = %e, "Bin-scan cache not written; the next build scans again");
                }
            }
            Err(e) => debug!(error = %e, "Bin-scan cache not serialized"),
        }
    }
    characters
}
