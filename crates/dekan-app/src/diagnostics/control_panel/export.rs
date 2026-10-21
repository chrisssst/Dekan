use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::{debug, warn};

use super::redact;

const DIAGNOSTIC_LOG_DAYS: u64 = 3;
const DIAGNOSTIC_MAX_BYTES: u64 = 256 * 1024 * 1024;

struct Skipped {
    count: usize,
    first: Option<String>,
}

impl Skipped {
    fn note(&mut self, what: impl FnOnce() -> String) {
        self.count += 1;
        self.first.get_or_insert_with(what);
    }
}

fn recent_logs(
    logs_dir: &Path,
    cutoff: SystemTime,
    skipped: &mut Skipped,
) -> Result<Vec<PathBuf>, String> {
    let mut logs = Vec::new();
    let entries = std::fs::read_dir(logs_dir)
        .map_err(|e| format!("logs folder {} unreadable: {e}", logs_dir.display()))?;
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(e) => {
                skipped.note(|| format!("{}: {e}", logs_dir.display()));
                continue;
            }
        };
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("dekan.log"))
        {
            continue;
        }
        match std::fs::metadata(&path).and_then(|m| m.modified()) {
            Ok(modified) if modified >= cutoff => logs.push(path),
            Ok(_) => {}
            Err(e) => skipped.note(|| format!("{}: {e}", path.display())),
        }
    }
    logs.sort();
    Ok(logs)
}

fn generated_manifests(data_dir: &Path, skipped: &mut Skipped) -> Vec<(String, PathBuf)> {
    let mods = match std::fs::read_dir(data_dir.join("mods")) {
        Ok(mods) => mods,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(error = %e, "The mods folder could not be read; the export carries no skin manifests");
            }
            return Vec::new();
        }
    };
    let mut generated = Vec::new();
    for entry in mods {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                skipped.note(|| format!("{}: {e}", data_dir.join("mods").display()));
                continue;
            }
        };
        let manifest = entry.path().join("META").join("manifest.json");
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            skipped.note(|| format!("{}: name is not text", entry.path().display()));
            continue;
        };
        if manifest.is_file() {
            generated.push((format!("manifests/{name}.json"), manifest));
        }
    }
    generated.sort();
    generated
}

fn write_zip(
    partial: &Path,
    files: &[(String, PathBuf)],
    profile: Option<&str>,
) -> Result<(), String> {
    use std::io::Write;

    let file = std::fs::File::create(partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut total = 0u64;
    for (name, path) in files {
        let mut bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !name.starts_with("screenshots/") {
            bytes = redact::redact(&String::from_utf8_lossy(&bytes), profile).into_bytes();
        }
        total += bytes.len() as u64;
        if total > DIAGNOSTIC_MAX_BYTES {
            return Err(format!(
                "logs larger than the diagnostic limit ({total} bytes after {name})"
            ));
        }
        zip.start_file(name.as_str(), options)
            .map_err(|e| format!("{name}: {e}"))?;
        zip.write_all(&bytes).map_err(|e| format!("{name}: {e}"))?;
    }
    zip.finish()
        .map_err(|e| format!("{}: {e}", partial.display()))?;
    Ok(())
}

pub fn export_diagnostics(
    logs_dir: &Path,
    now: SystemTime,
    screenshots: &[PathBuf],
) -> Result<PathBuf, String> {
    let cutoff = now
        .checked_sub(Duration::from_secs(DIAGNOSTIC_LOG_DAYS * 24 * 60 * 60))
        .unwrap_or(UNIX_EPOCH);
    let mut skipped = Skipped {
        count: 0,
        first: None,
    };
    let logs = recent_logs(logs_dir, cutoff, &mut skipped)?;
    if logs.is_empty() {
        return Err("no Dekan log from the last days".into());
    }
    let mut files: Vec<(String, PathBuf)> = Vec::with_capacity(logs.len());
    for log in logs {
        match log.file_name().and_then(|n| n.to_str()) {
            Some(name) => files.push((name.to_owned(), log.clone())),
            None => skipped.note(|| format!("{}: name is not text", log.display())),
        }
    }
    if let Some(data_dir) = logs_dir.parent() {
        let overlay = data_dir.join("overlay_manifest.json");
        if overlay.is_file() {
            files.push(("manifests/overlay_manifest.json".into(), overlay));
        } else {
            debug!(file = %overlay.display(), "No overlay manifest to export");
        }
        files.extend(generated_manifests(data_dir, &mut skipped));
    }
    for shot in screenshots {
        match shot.file_name().and_then(|n| n.to_str()) {
            Some(name) => files.push((format!("screenshots/{name}"), shot.clone())),
            None => skipped.note(|| format!("{}: name is not text", shot.display())),
        }
    }
    if let Some(first) = skipped.first {
        warn!(skipped = skipped.count, first = %first, "Some files could not be added to the diagnostics export");
    }
    let stamp = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let target = logs_dir.join(format!("dekan-diagnostics-{stamp}.zip"));
    let partial = logs_dir.join(format!("dekan-diagnostics-{stamp}.zip.partial"));
    let profile = std::env::var("USERPROFILE")
        .inspect_err(|e| {
            debug!(error = %e, "USERPROFILE is not readable; only user folders are masked in the export");
        })
        .ok();
    if let Err(e) = write_zip(&partial, &files, profile.as_deref()) {
        let _ = std::fs::remove_file(&partial); // ignore-ok: the export error is what gets reported
        return Err(e);
    }
    std::fs::rename(&partial, &target).map_err(|e| {
        let _ = std::fs::remove_file(&partial); // ignore-ok: the rename error is what gets reported
        format!("{}: {e}", target.display())
    })?;
    Ok(target)
}
