use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dekan_inject::mod_compat::{Repairer, check, game_hash_set, mod_hashes, mod_wads};
use dekan_inject::overlay_builder::get_or_index_game;
use dekan_platform::fs::{atomic_write, get_disk_free_space, safe_extract_zip};
use dekan_platform::game_version::game_exe;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::mods_store::{CUSTOM_MOD_LIMITS, STAGING_HEADROOM, unpack_modpkg, unpacked_size};

const VERDICTS_FILE: &str = "mod_repair.json";

const ORIGINALS_DIR: &str = "mod_originals";

const WORK_DIR: &str = "mod_repair_work";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Compatible,
    Repaired,
    RepairedOnEachInjection,
    Incompatible,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Verdicts {
    game: String,
    mods: BTreeMap<String, (String, Verdict)>,
    #[serde(default)]
    kept_original: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanSummary {
    pub repaired: usize,
    pub refused: usize,
}

#[derive(Debug, Clone, Default)]
pub struct CustomModsNotice(std::sync::Arc<std::sync::Mutex<Option<ScanSummary>>>);

impl CustomModsNotice {
    pub fn set(&self, summary: ScanSummary) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(summary);
        }
    }

    #[must_use]
    pub fn summary(&self) -> Option<ScanSummary> {
        self.0.lock().ok().and_then(|slot| *slot)
    }
}

impl RepairReport {
    #[must_use]
    pub fn this_run(&self) -> ScanSummary {
        ScanSummary {
            repaired: self.repaired.len(),
            refused: self.incompatible.len(),
        }
    }
}

static SCANNING: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

struct ScanGuard(PathBuf);

impl ScanGuard {
    fn try_enter(state_dir: &Path) -> Option<Self> {
        let mut busy = SCANNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if busy.iter().any(|dir| dir == state_dir) {
            return None;
        }
        busy.push(state_dir.to_path_buf());
        Some(Self(state_dir.to_path_buf()))
    }
}

impl Drop for ScanGuard {
    fn drop(&mut self) {
        let mut busy = SCANNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        busy.retain(|dir| *dir != self.0);
    }
}

fn totals(verdicts: &Verdicts) -> ScanSummary {
    let mut summary = ScanSummary {
        repaired: 0,
        refused: 0,
    };
    for (_, verdict) in verdicts.mods.values() {
        match verdict {
            Verdict::Repaired | Verdict::RepairedOnEachInjection => summary.repaired += 1,
            Verdict::Incompatible => summary.refused += 1,
            Verdict::Compatible => {}
        }
    }
    summary
}

#[derive(Debug, Default)]
pub struct RepairReport {
    pub checked: usize,
    pub unchanged_since_last_run: usize,
    pub kept_original: usize,
    pub repaired: Vec<PathBuf>,
    pub incompatible: Vec<PathBuf>,
    pub ran: bool,
    pub totals: Option<ScanSummary>,
}

struct Context<'a> {
    root: &'a Path,
    state: &'a Path,
    originals: PathBuf,
    work: PathBuf,
    hashes: &'a std::collections::HashSet<u64>,
}

fn repair_one(
    path: &Path,
    package: Package,
    cx: &Context,
    repairer: &mut Repairer,
) -> Result<Verdict, String> {
    if cx.work.exists() {
        std::fs::remove_dir_all(&cx.work).map_err(|e| e.to_string())?;
    }
    if package != Package::Directory {
        let needed = unpacked_size(path)
            .saturating_mul(2)
            .saturating_add(STAGING_HEADROOM);
        if let Ok(free) = get_disk_free_space(cx.state) {
            if free < needed {
                return Err(format!(
                    "not enough free disk space ({free} bytes) to repair a mod that needs {needed}"
                ));
            }
        }
    }
    match package {
        Package::Archive => {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            safe_extract_zip(std::io::BufReader::new(file), &cx.work, &CUSTOM_MOD_LIMITS)
                .map_err(|e| e.to_string())?;
        }
        Package::Directory => {
            let wad_dir = child_ci(path, "WAD").ok_or("mod folder without WAD")?;
            copy_tree(&wad_dir, &cx.work.join("WAD")).map_err(|e| e.to_string())?;
        }
        Package::ModPkg => {
            let name = path
                .file_stem()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            unpack_modpkg(path, &name, &cx.work)?;
        }
    }

    let wads = mod_wads(&cx.work);
    let own = mod_hashes(&wads);
    let mut changed = Vec::new();
    let mut relinked = 0;
    let mut dangling = Vec::new();
    let mut formats = std::collections::BTreeSet::new();
    for wad in &wads {
        match repairer.unknown_formats(wad) {
            Ok(found) => formats.extend(found),
            Err(e) => debug!(mod_path = %path.display(), error = %e, "Asset formats not checked"),
        }
        let repair = repairer.repair(wad, &own).map_err(|e| e.to_string())?;
        if !repair.relinks.is_empty() {
            relinked += repair.relinks.len();
            debug!(mod_path = %path.display(), wad = %wad.display(), relinks = ?repair.relinks, "Custom mod references relinked");
            changed.extend(repair.files);
        }
        dangling.extend(
            check(wad, cx.hashes, &own)
                .map_err(|e| e.to_string())?
                .dangling,
        );
    }

    if !formats.is_empty() {
        let formats: Vec<String> = formats.iter().map(ToString::to_string).collect();
        warn!(
            mod_path = %path.display(),
            formats = ?formats,
            "Custom mod carries asset formats the installed game no longer uses; those parts may not show in game"
        );
    }
    if !dangling.is_empty() {
        warn!(
            mod_path = %path.display(),
            dangling = ?dangling,
            "Custom mod links files the installed game no longer has and has no single successor; it stays as it is and is not injected"
        );
        return Ok(Verdict::Incompatible);
    }
    if changed.is_empty() {
        return Ok(Verdict::Compatible);
    }

    match package {
        Package::ModPkg => {
            info!(
                mod_path = %path.display(),
                references = relinked,
                "Custom mod package needs repairs for the installed patch; they are applied to its prepared copy on each injection"
            );
            return Ok(Verdict::RepairedOnEachInjection);
        }
        Package::Archive => {
            let partial = PathBuf::from(format!("{}.partial", path.display()));
            let replaced = zip_tree(&cx.work, &partial).and_then(|()| {
                copy_to_originals(path, cx.root, &cx.originals).map_err(|e| e.to_string())?;
                std::fs::rename(&partial, path).map_err(|e| e.to_string())
            });
            if let Err(e) = replaced {
                let _ = std::fs::remove_file(&partial); // ignore-ok: the packing, backup or rename error is what gets reported
                return Err(e);
            }
        }
        Package::Directory => {
            let work_wad = cx.work.join("WAD");
            let wad_dir = child_ci(path, "WAD").ok_or("WAD folder vanished")?;
            for file in &changed {
                let relative = file.strip_prefix(&work_wad).map_err(|e| e.to_string())?;
                let target = wad_dir.join(relative);
                copy_to_originals(&target, cx.root, &cx.originals).map_err(|e| e.to_string())?;
                let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
                atomic_write(&target, &bytes, true).map_err(|e| e.to_string())?;
            }
        }
    }
    info!(
        mod_path = %path.display(),
        references = relinked,
        backup = %cx.originals.display(),
        "Custom mod repaired for the installed patch"
    );
    Ok(Verdict::Repaired)
}

pub fn repair_custom_mods(
    roots: &[PathBuf],
    game_dir: &Path,
    state_dir: &Path,
    cancelled: &dyn Fn() -> bool,
) -> RepairReport {
    let mut report = RepairReport::default();
    let Some(_guard) = ScanGuard::try_enter(state_dir) else {
        debug!("Custom mods are being restored; the startup check is skipped this time");
        return report;
    };
    let Some(exe_stamp) = stamp(&game_exe(game_dir)) else {
        warn!(game_dir = %game_dir.display(), "Custom mods not checked at startup: the game executable was not found");
        return report;
    };
    let game = match get_or_index_game(game_dir) {
        Ok(game) => game,
        Err(e) => {
            warn!(error = %e, "Custom mods not checked at startup: the game could not be indexed");
            return report;
        }
    };
    let hashes = game_hash_set(&game);
    let game_stamp = format!("{exe_stamp}|{}:{}", game.len(), hashes.len());

    let verdicts_path = state_dir.join(VERDICTS_FILE);
    let mut verdicts = load_verdicts(&verdicts_path);
    if verdicts.game != game_stamp {
        verdicts = Verdicts {
            game: game_stamp,
            mods: BTreeMap::new(),
            kept_original: std::mem::take(&mut verdicts.kept_original),
        };
    }

    let started = std::time::Instant::now();
    let mut seen = Vec::new();
    let mut interrupted = false;
    let mut repairer = Repairer::new(&game, &hashes);
    for root in roots {
        let mut mods = Vec::new();
        collect_mods(root, &mut mods);
        let cx = Context {
            root,
            state: state_dir,
            originals: state_dir.join(ORIGINALS_DIR),
            work: state_dir.join(WORK_DIR),
            hashes: &hashes,
        };
        for (path, package) in mods {
            if cancelled() {
                interrupted = true;
                break;
            }
            let key = path.display().to_string();
            seen.push(key.clone());
            let Some(before) = package_stamp(&path, package) else {
                continue;
            };
            if verdicts.kept_original.get(&key) == Some(&before) {
                report.kept_original += 1;
                continue;
            }
            verdicts.kept_original.remove(&key);
            if verdicts.mods.get(&key).is_some_and(|(s, _)| *s == before) {
                report.unchanged_since_last_run += 1;
                continue;
            }
            report.checked += 1;
            let verdict = match repair_one(&path, package, &cx, &mut repairer) {
                Ok(verdict) => verdict,
                Err(reason) => {
                    warn!(mod_path = %path.display(), reason = %reason, "Custom mod could not be checked against the installed patch; it is checked again next start");
                    continue;
                }
            };
            match verdict {
                Verdict::Repaired => report.repaired.push(path.clone()),
                Verdict::Incompatible => report.incompatible.push(path.clone()),
                Verdict::Compatible | Verdict::RepairedOnEachInjection => {}
            }
            if let Some(after) = package_stamp(&path, package) {
                verdicts.mods.insert(key, (after, verdict));
            }
        }
        if cx.work.exists() {
            let _ = std::fs::remove_dir_all(&cx.work); // ignore-ok: scratch extraction, recreated on the next run
        }
    }
    if !interrupted {
        verdicts.mods.retain(|k, _| seen.contains(k));
        verdicts.kept_original.retain(|k, _| seen.contains(k));
    }

    save_verdicts(&verdicts_path, &verdicts);
    report.ran = true;
    report.totals = Some(totals(&verdicts));
    info!(
        kept_original = report.kept_original,
        interrupted,
        checked = report.checked,
        unchanged = report.unchanged_since_last_run,
        repaired = report.repaired.len(),
        incompatible = report.incompatible.len(),
        elapsed_ms = started.elapsed().as_millis(),
        "Custom mods checked against the installed patch"
    );
    report
}

mod files;
mod restore;

use files::{
    Package, child_ci, collect_mods, copy_to_originals, copy_tree, files_in, load_verdicts,
    package_stamp, save_verdicts, stamp, zip_tree,
};
pub use restore::{RestoreOutcome, restore_originals};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod restore_tests;

#[cfg(test)]
mod edge_tests;
