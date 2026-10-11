use super::*;

pub(super) fn generate(
    case: &Case,
    game: &Path,
    mods: &Path,
    cache: &Path,
) -> Result<String, String> {
    match case {
        Case::Standard {
            alias, skin, base, ..
        } => StandardChampion::open(game, alias)
            .map_err(|e| e.to_string())?
            .with_cache_dir(cache)
            .build_mod(*skin, *base, mods)
            .map_err(|e| e.to_string()),
        Case::Classic {
            alias,
            classic_alias,
            skin,
            ..
        } => {
            let champion = ClassicChampion::open(game, alias)
                .map_err(|e| e.to_string())?
                .with_client_character(Some(classic_alias));
            let known = champion.jade_names_from_bins_cached(cache);
            champion
                .build_mod(*skin, &slots_for(None), &known, mods)
                .map_err(|e| e.to_string())
        }
    }
}

pub(super) fn overlay_wads(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".wad.client"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

pub fn run_case(case: &Case, game: &Path, scratch: &Path, cache: &Path) -> Outcome {
    let mut outcome = Outcome {
        label: case.label().to_owned(),
        ..Outcome::default()
    };
    let mods = scratch.join("mods");
    let overlay = scratch.join("overlay");
    let _ = std::fs::remove_dir_all(scratch); // ignore-ok: scratch folder may not exist yet
    if let Err(e) = std::fs::create_dir_all(&mods) {
        outcome.failures.push(format!("scratch folder: {e}"));
        return outcome;
    }

    let folder = match generate(case, game, &mods, cache) {
        Ok(folder) => folder,
        Err(e) => {
            outcome.failures.push(format!("generation: {e}"));
            return outcome;
        }
    };
    let started = std::time::Instant::now();
    if let Err(e) = overlay_builder::build(
        game,
        &mods,
        &overlay,
        std::slice::from_ref(&folder),
        &AtomicBool::new(false),
    ) {
        outcome.failures.push(format!("overlay build: {e}"));
        return outcome;
    }
    outcome.build_ms = started.elapsed().as_millis();

    let index = match overlay_builder::get_or_index_game(game) {
        Ok(index) => index,
        Err(e) => {
            outcome.failures.push(format!("game index: {e}"));
            return outcome;
        }
    };
    let rewritten: Vec<PathBuf> = overlay_wads(&overlay);
    outcome.overlay_wads = rewritten.len();
    if rewritten.is_empty() {
        outcome.failures.push("the overlay holds no WAD".into());
        return outcome;
    }
    let rewritten_rel: Vec<PathBuf> = rewritten
        .iter()
        .filter_map(|p| p.strip_prefix(&overlay).ok().map(Path::to_path_buf))
        .collect();

    let mut changed: BTreeMap<u64, (PathBuf, Vec<u8>)> = BTreeMap::new();
    for (overlay_wad, relative) in rewritten.iter().zip(&rewritten_rel) {
        let built = match WadFile::open(overlay_wad) {
            Ok(wad) => wad,
            Err(e) => {
                outcome
                    .failures
                    .push(format!("{}: opens: {e}", relative.display()));
                continue;
            }
        };
        let original = match WadFile::open(&game.join(relative)) {
            Ok(wad) => wad,
            Err(e) => {
                outcome
                    .failures
                    .push(format!("{}: game copy: {e}", relative.display()));
                continue;
            }
        };
        let game_entries: HashMap<u64, _> = original.toc().map(|e| (e.path_hash, e)).collect();
        for entry in built.toc() {
            if let Some(game_entry) = game_entries.get(&entry.path_hash) {
                if game_entry.compression == entry.compression
                    && game_entry.compressed_size == entry.compressed_size
                    && game_entry.uncompressed_size == entry.uncompressed_size
                    && game_entry.checksum == entry.checksum
                {
                    outcome.verbatim_entries += 1;
                    continue;
                }
            }
            let decoded = match built.read(entry.path_hash) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    outcome.failures.push(format!(
                        "{}: {:#x} vanished",
                        relative.display(),
                        entry.path_hash
                    ));
                    continue;
                }
                Err(e) => {
                    outcome.failures.push(format!(
                        "{}: {:#x} does not decode: {e}",
                        relative.display(),
                        entry.path_hash
                    ));
                    continue;
                }
            };
            match game_entries.get(&entry.path_hash) {
                Some(game_entry)
                    if original.read(entry.path_hash).ok().flatten().as_deref()
                        == Some(&decoded[..]) =>
                {
                    outcome.failures.push(format!(
                        "{}: {:#x} unchanged but re-encoded ({:?}/{} -> {:?}/{})",
                        relative.display(),
                        entry.path_hash,
                        game_entry.compression,
                        game_entry.compressed_size,
                        entry.compression,
                        entry.compressed_size
                    ));
                }
                _ => {
                    changed.insert(entry.path_hash, (relative.clone(), decoded));
                }
            }
        }
        for hash in game_entries.keys() {
            if built.entry(*hash).is_none() {
                outcome.failures.push(format!(
                    "{}: {hash:#x} dropped from the game's WAD",
                    relative.display()
                ));
            }
        }
    }
    outcome.changed_entries = changed.len();
    if changed.is_empty() {
        outcome
            .failures
            .push("no entry differs from the game: the skin would not change".into());
    }

    for (hash, (owner, bytes)) in &changed {
        for wad in index.values().filter(|w| w.contains(*hash)) {
            if rewritten_rel.iter().any(|r| r == &wad.relpath) || &wad.relpath == owner {
                continue;
            }
            let game_bytes = WadFile::open(&wad.path)
                .ok()
                .and_then(|w| w.read(*hash).ok().flatten());
            if game_bytes.as_deref() != Some(&bytes[..]) {
                outcome.failures.push(format!(
                    "{hash:#x} changed in {} but {} (mounted as the game has it) holds other bytes: Inconsistent",
                    owner.display(),
                    wad.relpath.display()
                ));
            }
        }
    }
    let _ = std::fs::remove_dir_all(scratch); // ignore-ok: scratch folder
    outcome
}
