use super::*;

pub(super) fn archive_stamp(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            format!("{}:{mtime}", meta.len())
        }
        Err(_) => String::new(),
    }
}

pub(crate) const STAGING_HEADROOM: u64 = 512 * 1024 * 1024;

pub(crate) fn unpacked_size(path: &Path) -> u64 {
    let on_disk = std::fs::metadata(path).map_or(0, |m| m.len());
    let declared = if is_modpkg_file(path) {
        read_modpkg(path).ok().map(|package| package.base_size())
    } else {
        std::fs::File::open(path)
            .ok()
            .and_then(|file| zip::ZipArchive::new(std::io::BufReader::new(file)).ok())
            .map(|mut archive| {
                (0..archive.len())
                    .filter_map(|i| archive.by_index_raw(i).ok().map(|entry| entry.size()))
                    .fold(0, u64::saturating_add)
            })
    };
    declared.unwrap_or(0).max(on_disk)
}

pub(super) fn is_modpkg_file(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8];
    std::fs::File::open(path).is_ok_and(|mut file| file.read_exact(&mut head).is_ok())
        && dekan_wad::modpkg::is_modpkg(&head)
}

pub(super) fn read_modpkg(
    path: &Path,
) -> Result<dekan_wad::modpkg::ModPkg, dekan_wad::error::WadError> {
    let file = std::fs::File::open(path).map_err(|source| dekan_wad::error::WadError::FileIo {
        path: path.display().to_string(),
        source,
    })?;
    dekan_wad::modpkg::ModPkg::read_index(std::io::BufReader::new(file))
}

pub(crate) fn unpack_modpkg(path: &Path, name: &str, into: &Path) -> Result<usize, String> {
    let failed = |e: &dyn std::fmt::Display| format!("mod package '{}': {e}", path.display());
    let package = read_modpkg(path).map_err(|e| failed(&e))?;
    let file = std::fs::File::open(path).map_err(|e| failed(&e))?;
    let wads = package
        .base_wads(std::io::BufReader::new(file))
        .map_err(|e| failed(&e))?;
    let wad_dir = into.join("WAD");
    std::fs::create_dir_all(&wad_dir).map_err(|e| failed(&e))?;
    std::fs::create_dir_all(into.join("META")).map_err(|e| failed(&e))?;
    let manifest = serde_json::json!({ "Name": name }).to_string();
    std::fs::write(into.join("META").join("info.json"), manifest).map_err(|e| failed(&e))?;
    for (stem, entries) in &wads {
        let mut writer = dekan_wad::WadWriter::default();
        for (hash, entry) in entries {
            writer.insert(*hash, entry.clone());
        }
        writer
            .write_to_file(&wad_dir.join(format!("{stem}.wad.client")), &|| false)
            .map_err(|e| failed(&e))?;
    }
    Ok(wads.len() + 1)
}

pub(super) fn stage_one(entry: &ModEntry, mods_dir: &Path) -> Result<String, String> {
    match entry.package {
        ModPackage::Directory => {
            let name = staged_name(&entry.id, "dir");
            let dst = mods_dir.join(&name);
            if dst.exists() {
                std::fs::remove_dir_all(&dst)
                    .map_err(|e| format!("could not clear '{}': {e}", dst.display()))?;
            }
            mirror_tree(&entry.path, &dst).map_err(|e| e.to_string())?;
            Ok(name)
        }
        ModPackage::Archive => {
            let name = staged_name(&entry.id, &archive_stamp(&entry.path));
            let dst = mods_dir.join(&name);
            if is_valid_mod_dir(&dst) {
                debug!(mod_id = %entry.id, staged = %name, "Archive already extracted; reusing it");
                return Ok(name);
            }

            let unpacked = unpacked_size(&entry.path);
            if unpacked > CUSTOM_MOD_LIMITS.max_total_bytes {
                return Err(format!(
                    "the archive declares {unpacked} bytes, more than a mod may unpack to"
                ));
            }
            if let Ok(free) = dekan_platform::fs::get_disk_free_space(mods_dir) {
                if free < unpacked.saturating_add(STAGING_HEADROOM) {
                    return Err(format!(
                        "not enough free disk space ({free} bytes) to unpack {unpacked} bytes"
                    ));
                }
            }

            let partial = mods_dir.join(format!("{name}.partial"));
            if partial.exists() {
                std::fs::remove_dir_all(&partial)
                    .map_err(|e| format!("could not clear '{}': {e}", partial.display()))?;
            }
            if dst.exists() {
                std::fs::remove_dir_all(&dst)
                    .map_err(|e| format!("could not clear '{}': {e}", dst.display()))?;
            }

            let files = if is_modpkg_file(&entry.path) {
                unpack_modpkg(&entry.path, &entry.name, &partial)?
            } else {
                let file = std::fs::File::open(&entry.path)
                    .map_err(|e| format!("could not open '{}': {e}", entry.path.display()))?;
                safe_extract_zip(std::io::BufReader::new(file), &partial, &CUSTOM_MOD_LIMITS)
                    .map_err(|e| e.to_string())?
            };

            if !is_valid_mod_dir(&partial) {
                // ignore-ok: removing our own rejected extraction; the refusal itself is returned.
                let _ = std::fs::remove_dir_all(&partial);
                return Err(
                    "archive is not a mod the overlay builder accepts (META/info.json plus WAD/ or RAW/)"
                        .into(),
                );
            }
            std::fs::rename(&partial, &dst)
                .map_err(|e| format!("could not move the extraction into place: {e}"))?;
            info!(mod_id = %entry.id, staged = %name, files, "Custom mod archive extracted");
            Ok(name)
        }
    }
}

pub fn stage_selected(
    catalog: &ModCatalog,
    selection: &ModSelection,
    champion_id: Option<ChampionId>,
    mods_dir: &Path,
) -> Vec<String> {
    if let Err(e) = std::fs::create_dir_all(mods_dir) {
        error!(dir = %mods_dir.display(), error = %e, "Staging directory unavailable; no custom mod will load");
        return Vec::new();
    }

    let mut staged = Vec::new();
    for id in selection.ordered_ids(champion_id) {
        let Some(entry) = catalog.find(id) else {
            warn!(mod_id = %id, "Selected custom mod is no longer on disk; skipping it");
            continue;
        };
        let started = std::time::Instant::now();
        match stage_one(entry, mods_dir) {
            Ok(name) => {
                info!(
                    mod_id = %entry.id,
                    category = ?entry.category,
                    source = ?entry.source,
                    staged = %name,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Custom mod staged"
                );
                staged.push(name);
            }
            Err(reason) => {
                warn!(mod_id = %entry.id, reason = %reason, "Custom mod could not be staged; skipping it");
            }
        }
    }

    remove_stale_staged(mods_dir, &staged);
    staged
}

pub(super) fn remove_stale_staged(mods_dir: &Path, keep: &[String]) {
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        return;
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with(STAGED_PREFIX) || keep.iter().any(|k| k == &name) {
            continue;
        }
        match std::fs::remove_dir_all(entry.path()) {
            Ok(()) => removed += 1,
            Err(e) => debug!(staged = %name, error = %e, "Stale staged mod could not be removed"),
        }
    }
    if removed > 0 {
        debug!(removed, "Stale staged custom mods removed");
    }
}
