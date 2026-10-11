use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Package {
    Archive,
    Directory,
    ModPkg,
}

pub(super) fn stamp(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{}:{mtime}", meta.len()))
}

pub(super) fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
}

pub(super) fn files_in(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)?.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

pub(super) fn directory_stamp(dir: &Path) -> Option<String> {
    let wad_dir = child_ci(dir, "WAD")?;
    let parts: Vec<String> = files_in(&wad_dir)
        .ok()?
        .iter()
        .filter_map(|f| stamp(f))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let mut hasher = std::hash::DefaultHasher::new();
    std::hash::Hash::hash(&parts, &mut hasher);
    Some(format!(
        "{}:{:016x}",
        parts.len(),
        std::hash::Hasher::finish(&hasher)
    ))
}

pub(super) fn package_stamp(path: &Path, package: Package) -> Option<String> {
    match package {
        Package::Archive | Package::ModPkg => stamp(path),
        Package::Directory => directory_stamp(path),
    }
}

pub(super) fn collect_mods(dir: &Path, out: &mut Vec<(PathBuf, Package)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| !e.file_type().is_ok_and(|kind| kind.is_symlink()))
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if dekan_core::mods::is_valid_mod_dir(&path) {
                out.push((path, Package::Directory));
            } else {
                collect_mods(&path, out);
            }
            continue;
        }
        let lower = path.to_string_lossy().to_ascii_lowercase();
        if lower.ends_with(".fantome") || lower.ends_with(".zip") {
            out.push((path, Package::Archive));
        } else if lower.ends_with(".modpkg") {
            out.push((path, Package::ModPkg));
        }
    }
}

pub(super) fn copy_to_originals(
    original: &Path,
    root: &Path,
    originals: &Path,
) -> std::io::Result<()> {
    let relative = original.strip_prefix(root).unwrap_or(original);
    let backup = originals.join(relative);
    if let Some(parent) = backup.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = PathBuf::from(format!("{}.partial", backup.display()));
    std::fs::copy(original, &partial)?;
    std::fs::rename(&partial, &backup)
}

pub(super) fn zip_tree(dir: &Path, target: &Path) -> Result<(), String> {
    let files = files_in(dir).map_err(|e| e.to_string())?;

    let file = std::fs::File::create(target).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    for path in files {
        let name = path
            .strip_prefix(dir)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        let mut source = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        std::io::copy(&mut source, &mut zip).map_err(|e| e.to_string())?;
    }
    let written = zip.finish().map_err(|e| e.to_string())?;
    let file = written.into_inner().map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

pub(super) fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    for file in files_in(from)? {
        let Ok(relative) = file.strip_prefix(from) else {
            continue;
        };
        let target = to.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&file, &target)?;
    }
    Ok(())
}

pub(super) fn load_verdicts(path: &Path) -> Verdicts {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(super) fn save_verdicts(path: &Path, verdicts: &Verdicts) {
    match serde_json::to_vec_pretty(verdicts) {
        Ok(bytes) => {
            if let Err(e) = atomic_write(path, &bytes, false) {
                warn!(error = %e, "Custom mod verdicts not saved; every mod is checked again next start");
            }
        }
        Err(e) => warn!(error = %e, "Custom mod verdicts not serialized"),
    }
}
