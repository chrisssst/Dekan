use super::*;

pub(super) fn overwrite_shared(
    target: &mut BTreeMap<u64, WriterEntry>,
    source: &BTreeMap<u64, WriterEntry>,
    winner: &str,
) -> usize {
    let mut conflicts = 0usize;
    for (hash, entry) in target.iter_mut() {
        if let Some(newer) = source.get(hash) {
            if newer.checksum != entry.checksum {
                conflicts += 1;
                debug!(
                    path_hash = format_args!("{hash:#018x}"),
                    winner, "Conflicting mod entry; the newer one wins"
                );
            }
            *entry = newer.clone();
        }
    }
    conflicts
}

pub(super) fn base_store_for(overlay_dir: &Path) -> Option<PathBuf> {
    overlay_dir
        .parent()
        .map(|parent| parent.join(BASE_STORE_DIR))
}

pub(super) fn move_with_stamp(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(base_stamp_path(to)); // ignore-ok: an older stamp at the destination is replaced below
    std::fs::rename(from, to)?;
    std::fs::rename(base_stamp_path(from), base_stamp_path(to))
}

pub(super) fn restore_base(stored: &Path, out: &Path) {
    if out.exists() || !stored.is_file() || !base_stamp_path(stored).is_file() {
        return;
    }
    if let Err(e) = move_with_stamp(stored, out) {
        debug!(file = %stored.display(), error = %e, "Kept game WAD copy not restored; it is copied again");
    }
}

#[derive(Debug, Default)]
pub(super) struct Strays {
    pub removed: usize,
    pub stuck: usize,
    pub unlisted: usize,
    first_error: Option<String>,
}

impl Strays {
    fn fail(&mut self, path: &Path, e: &std::io::Error) {
        self.first_error
            .get_or_insert_with(|| format!("{}: {e}", path.display()));
    }
}

pub(super) fn remove_strays(dir: &Path, keep: &HashSet<&str>, base_store: Option<&Path>) -> Strays {
    let mut strays = Strays::default();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = match std::fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(e) => {
                strays.unlisted += 1;
                strays.fail(&current, &e);
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    strays.unlisted += 1;
                    strays.fail(&current, &e);
                    continue;
                }
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind) if kind.is_file() => {
                    let stray = name.ends_with(".wad.client.partial")
                        || (name.ends_with(".wad.client")
                            && !keep.contains(mount_name(&name).as_str()));
                    let kept_copy = base_store.zip(path.strip_prefix(dir).ok()).filter(|_| {
                        name.ends_with(".wad.client") && base_stamp_path(&path).is_file()
                    });
                    if stray {
                        if let Some((store, relative)) = kept_copy {
                            match move_with_stamp(&path, &store.join(relative)) {
                                Ok(()) => {
                                    strays.removed += 1;
                                    continue;
                                }
                                Err(e) if !path.exists() => {
                                    debug!(file = %path.display(), error = %e, "Game WAD copy kept without its stamp; it is copied again");
                                    strays.removed += 1;
                                    continue;
                                }
                                Err(e) => {
                                    debug!(file = %path.display(), error = %e, "Game WAD copy not kept; it is removed")
                                }
                            }
                        }
                        let _ = std::fs::remove_file(base_stamp_path(&path)); // ignore-ok: the stamp only describes the WAD removed below
                        match std::fs::remove_file(&path) {
                            Ok(()) => strays.removed += 1,
                            Err(e) => {
                                strays.stuck += 1;
                                strays.fail(&path, &e);
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    strays.unlisted += 1;
                    strays.fail(&path, &e);
                }
            }
        }
    }
    if strays.stuck > 0 || strays.unlisted > 0 {
        warn!(
            overlay_dir = %dir.display(),
            stuck = strays.stuck,
            unlisted = strays.unlisted,
            first_error = strays.first_error.as_deref().unwrap_or_default(),
            "Stray overlay WADs could not be removed or listed; the game may load them this match"
        );
    }
    strays
}
