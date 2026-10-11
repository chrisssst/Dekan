use super::*;

pub(super) fn index_mod(mod_dir: &Path, name: &str) -> Result<ModIndex, InjectError> {
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

pub(super) fn collect_mod_wads(dir: &Path, mod_name: &str, out: &mut Vec<PathBuf>) {
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

pub(super) fn retype_stale_bins(
    game: &GameIndexMap,
    index: &mut ModIndex,
) -> Result<(), InjectError> {
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

pub(super) fn game_field_shapes(
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

pub(super) fn read_mod_wad(
    path: &Path,
) -> Result<BTreeMap<u64, WriterEntry>, dekan_wad::error::WadError> {
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

pub(super) fn pack_folder(
    dir: &Path,
) -> Result<BTreeMap<u64, WriterEntry>, dekan_wad::error::WadError> {
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

pub(super) fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
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

pub(super) fn resolve_inside(index: &mut ModIndex) {
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

pub(super) fn resolve_against(older: &mut ModIndex, newer: &ModIndex) {
    for mount in older.mounts.values_mut() {
        for other in newer.mounts.values() {
            overwrite_shared(&mut mount.entries, &other.entries, &newer.name);
        }
    }
}

pub(super) fn decoded_mod_entry(entry: &WriterEntry) -> Option<Vec<u8>> {
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

pub(super) fn drop_entries_identical_to_game(
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
