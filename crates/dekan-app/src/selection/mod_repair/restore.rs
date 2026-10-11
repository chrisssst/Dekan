use super::*;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub busy: bool,
    pub restored: usize,
    pub failed: Vec<String>,
}

pub fn restore_originals(roots: &[PathBuf], state_dir: &Path) -> RestoreOutcome {
    let mut outcome = RestoreOutcome::default();
    let Some(_guard) = ScanGuard::try_enter(state_dir) else {
        outcome.busy = true;
        return outcome;
    };
    let originals = state_dir.join(ORIGINALS_DIR);
    let verdicts_path = state_dir.join(VERDICTS_FILE);
    let mut verdicts = load_verdicts(&verdicts_path);
    for root in roots {
        let mut mods = Vec::new();
        collect_mods(root, &mut mods);
        for (path, package) in mods {
            let key = path.display().to_string();
            let Some((repaired_stamp, Verdict::Repaired)) = verdicts.mods.get(&key).cloned() else {
                continue;
            };
            if package_stamp(&path, package).as_ref() != Some(&repaired_stamp) {
                debug!(mod_path = %path.display(), "Custom mod changed since it was repaired; its old original is not put back");
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let backup_root = originals.join(relative);
            let backups = if backup_root.is_dir() {
                files_in(&backup_root).unwrap_or_default()
            } else if backup_root.is_file() {
                vec![backup_root.clone()]
            } else {
                Vec::new()
            };
            if backups.is_empty() {
                continue;
            }
            let mut moved = 0;
            for backup in &backups {
                let Ok(inside) = backup.strip_prefix(&originals) else {
                    continue;
                };
                let target = root.join(inside);
                match std::fs::rename(backup, &target) {
                    Ok(()) => moved += 1,
                    Err(e) => outcome
                        .failed
                        .push(format!("could not put back '{}': {e}", target.display())),
                }
            }
            if moved == 0 {
                continue;
            }
            verdicts.mods.remove(&key);
            if let Some(current) = package_stamp(&path, package) {
                verdicts.kept_original.insert(key, current);
            }
            outcome.restored += 1;
        }
    }
    save_verdicts(&verdicts_path, &verdicts);
    info!(
        mods = outcome.restored,
        failed = outcome.failed.len(),
        "Custom mods restored to their original files at the user's request"
    );
    outcome
}
