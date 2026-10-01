use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use dekan_core::mods::{
    ModCatalog, ModCategory, ModEntry, ModPackage, ModRoot, ModSelection, ModSource, STAGED_PREFIX,
    is_valid_mod_dir, scan_catalog, staged_name,
};
use dekan_core::selection::ChampionId;
use dekan_platform::fs::{ExtractLimits, atomic_write, mirror_tree, safe_extract_zip};
use tracing::{debug, error, info, warn};

const SELECTION_FILE: &str = "mods_selection.json";

const SELECTION_VERSION: u32 = 1;

const CUSTOM_MOD_LIMITS: ExtractLimits = ExtractLimits {
    max_total_bytes: 16 * 1024 * 1024 * 1024,
    max_single_file_bytes: 8 * 1024 * 1024 * 1024,
    max_entries: 200_000,
    max_path_len: dekan_platform::fs::MAX_PATH_CHARS,
};

#[must_use]
pub fn dekan_mods_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("custom_mods")
}

#[must_use]
pub fn mod_roots(app_data_dir: &Path) -> Vec<ModRoot> {
    let own = dekan_mods_root(app_data_dir);
    for category in ModCategory::ALL {
        let dir = own.join(category.folder());
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!(dir = %dir.display(), error = %e, "Could not create a custom mods folder");
        }
    }

    report_misplaced(&own);

    vec![ModRoot {
        path: own,
        source: ModSource::Dekan,
    }]
}

fn report_misplaced(own: &Path) {
    let Ok(entries) = std::fs::read_dir(own) else {
        return;
    };
    let misplaced: Vec<String> = entries
        .flatten()
        .filter(|entry| {
            let path = entry.path();
            let is_archive = path.is_file()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    e.eq_ignore_ascii_case("fantome") || e.eq_ignore_ascii_case("zip")
                });
            is_archive || (path.is_dir() && is_valid_mod_dir(&path))
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    if !misplaced.is_empty() {
        warn!(
            root = %own.display(),
            mods = ?misplaced,
            "Mods outside a category folder are not offered; move each into skins, maps, fonts, \
             announcers, ui, voiceover, loading_screen, vfx, sfx or others"
        );
    }
}

#[must_use]
pub fn targeted_aliases(entry: &ModEntry) -> BTreeSet<String> {
    use dekan_wad::fantome::{wad_mount_alias, wad_name_in_path, wad_names_in_archive};

    let names = match entry.package {
        ModPackage::Archive => match std::fs::File::open(&entry.path) {
            Ok(file) => match wad_names_in_archive(std::io::BufReader::new(file)) {
                Ok(names) => names,
                Err(e) => {
                    debug!(mod_path = %entry.path.display(), error = %e, "Mod archive could not be listed");
                    BTreeSet::new()
                }
            },
            Err(e) => {
                debug!(mod_path = %entry.path.display(), error = %e, "Mod archive could not be opened");
                BTreeSet::new()
            }
        },
        ModPackage::Directory => {
            let mut names = BTreeSet::new();
            collect_wad_names(&entry.path, "", 0, &mut names, &wad_name_in_path);
            names
        }
    };
    names
        .iter()
        .map(|name| wad_mount_alias(name).to_ascii_lowercase())
        .collect()
}

fn collect_wad_names(
    dir: &Path,
    relative: &str,
    depth: usize,
    out: &mut BTreeSet<String>,
    name_of: &dyn Fn(&str) -> Option<String>,
) {
    const MAX_DEPTH: usize = 3;
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        if let Some(wad) = name_of(&path) {
            out.insert(wad);
        } else if entry.path().is_dir() {
            collect_wad_names(&entry.path(), &path, depth + 1, out, name_of);
        }
    }
}

#[must_use]
pub fn belongs_to_alias(entry: &ModEntry, alias: Option<&str>) -> bool {
    alias.is_some_and(|alias| targeted_aliases(entry).contains(&alias.to_ascii_lowercase()))
}

pub async fn load_mod_catalog(
    roots: Vec<ModRoot>,
    champion_id: Option<ChampionId>,
    alias: Option<String>,
) -> ModCatalog {
    let scan = move || {
        scan_catalog(&roots, champion_id, &|entry| {
            belongs_to_alias(entry, alias.as_deref())
        })
    };
    match tokio::task::spawn_blocking(scan).await {
        Ok(catalog) => catalog,
        Err(e) => {
            warn!(error = %e, "Mod scan task failed; no custom mod will be offered");
            ModCatalog::default()
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedSelection {
    version: u32,
    selection: ModSelection,
}

#[must_use]
pub fn load_selection(state_dir: &Path) -> ModSelection {
    let path = state_dir.join(SELECTION_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ModSelection::default(),
        Err(e) => {
            warn!(file = %path.display(), error = %e, "Mod selection could not be read; starting empty");
            return ModSelection::default();
        }
    };

    match serde_json::from_slice::<PersistedSelection>(&bytes) {
        Ok(persisted) => {
            if persisted.version != SELECTION_VERSION {
                warn!(
                    file = %path.display(),
                    version = persisted.version,
                    expected = SELECTION_VERSION,
                    "Mod selection written by another version; reading it as-is"
                );
            }
            let selection = persisted.selection;
            info!(
                map = ?selection.map,
                font = ?selection.font,
                announcer = ?selection.announcer,
                others = selection.others.len(),
                skin_mods = selection.skin.len(),
                "Custom mod selection restored"
            );
            selection
        }
        Err(e) => {
            let aside = path.with_extension("json.unreadable");
            let moved = std::fs::rename(&path, &aside);
            warn!(
                file = %path.display(),
                moved_to = %aside.display(),
                moved = moved.is_ok(),
                error = %e,
                "Mod selection file is not valid; it was set aside and the selection starts empty"
            );
            ModSelection::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportRefusal {
    UnsupportedExtension,
    NotAModPackage(String),
    NoManifest,
    NoContent,

    NoChampion,
    Io(String),
}

impl std::fmt::Display for ImportRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedExtension => write!(f, "only .fantome and .zip mods can be imported"),
            Self::NotAModPackage(e) => write!(f, "not a mod package: {e}"),
            Self::NoManifest => write!(f, "missing META/info.json manifest"),
            Self::NoContent => write!(f, "the package has no WAD/ or RAW/ content"),
            Self::NoChampion => write!(
                f,
                "the package does not say which champion it is for and no champion is selected"
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl ImportRefusal {
    /// The reason in the user's language.
    #[must_use]
    pub fn describe(&self, text: &dekan_platform::i18n::Text) -> String {
        use dekan_platform::i18n::fill;
        match self {
            Self::UnsupportedExtension => text.import_unsupported_extension.to_owned(),
            Self::NotAModPackage(e) => fill(text.import_not_a_mod, "error", e),
            Self::NoManifest => text.import_no_manifest.to_owned(),
            Self::NoContent => text.import_no_content.to_owned(),
            Self::NoChampion => text.import_no_champion.to_owned(),
            Self::Io(e) => fill(text.import_io_error, "error", e),
        }
    }
}

const IMPORT_NAME_MAX: usize = 60;

pub fn import_archive(
    own_root: &Path,
    category: ModCategory,
    champion_id: Option<ChampionId>,
    source: &Path,
) -> Result<PathBuf, ImportRefusal> {
    let extension = source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| e == "fantome" || e == "zip")
        .ok_or(ImportRefusal::UnsupportedExtension)?;

    let open = || {
        std::fs::File::open(source)
            .map_err(|e| ImportRefusal::Io(format!("cannot open the file: {e}")))
    };
    let shape = dekan_wad::fantome::mod_archive_shape(open()?)
        .map_err(|e| ImportRefusal::NotAModPackage(e.to_string()))?;
    if !shape.manifest {
        return Err(ImportRefusal::NoManifest);
    }
    if !shape.content {
        return Err(ImportRefusal::NoContent);
    }

    let mut dir = own_root.join(category.folder());
    if category == ModCategory::Skin {
        let names_a_champion =
            dekan_wad::fantome::wad_names_in_archive(std::io::BufReader::new(open()?))
                .map(|names| !names.is_empty())
                .unwrap_or(false);
        if !names_a_champion {
            let champion = champion_id.ok_or(ImportRefusal::NoChampion)?;
            dir = dir.join(champion.to_string());
        }
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| ImportRefusal::Io(format!("cannot create {}: {e}", dir.display())))?;

    let stem: String = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " _-().".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(IMPORT_NAME_MAX)
        .collect();
    let stem = stem.trim_matches(|c: char| c == ' ' || c == '.').to_owned();
    let stem = if stem.is_empty() {
        "mod".to_owned()
    } else {
        stem
    };
    let destination = (1..)
        .map(|n| {
            let name = if n == 1 {
                format!("{stem}.{extension}")
            } else {
                format!("{stem} ({n}).{extension}")
            };
            dir.join(name)
        })
        .find(|path| !path.exists())
        .ok_or_else(|| ImportRefusal::Io("no free file name".into()))?;

    let partial = destination.with_extension(format!("{extension}.partial"));
    std::fs::copy(source, &partial).map_err(|e| {
        let _ = std::fs::remove_file(&partial); // ignore-ok: best-effort cleanup of the half-copy; the copy error is what gets reported
        ImportRefusal::Io(format!("copy failed: {e}"))
    })?;
    std::fs::rename(&partial, &destination).map_err(|e| {
        let _ = std::fs::remove_file(&partial); // ignore-ok: best-effort cleanup; the rename error is what gets reported
        ImportRefusal::Io(format!("could not move the copy into place: {e}"))
    })?;
    Ok(destination)
}

pub fn save_selection(state_dir: &Path, selection: &ModSelection) {
    let path = state_dir.join(SELECTION_FILE);
    let persisted = PersistedSelection {
        version: SELECTION_VERSION,
        selection: selection.clone(),
    };
    let bytes = match serde_json::to_vec_pretty(&persisted) {
        Ok(bytes) => bytes,
        Err(e) => {
            error!(error = %e, "Mod selection could not be serialized; it will not survive a restart");
            return;
        }
    };
    match atomic_write(&path, &bytes, true) {
        Ok(()) => debug!(file = %path.display(), "Mod selection saved"),
        Err(e) => {
            warn!(file = %path.display(), error = %e, "Mod selection could not be saved");
        }
    }
}

fn archive_stamp(path: &Path) -> String {
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

fn stage_one(entry: &ModEntry, mods_dir: &Path) -> Result<String, String> {
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

            let archive_len = std::fs::metadata(&entry.path).map(|m| m.len()).unwrap_or(0);
            if let Ok(free) = dekan_platform::fs::get_disk_free_space(mods_dir) {
                if free < archive_len.saturating_mul(2) {
                    return Err(format!(
                        "not enough free disk space ({free} bytes) to extract a {archive_len}-byte archive"
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

            let file = std::fs::File::open(&entry.path)
                .map_err(|e| format!("could not open '{}': {e}", entry.path.display()))?;
            let files =
                safe_extract_zip(std::io::BufReader::new(file), &partial, &CUSTOM_MOD_LIMITS)
                    .map_err(|e| e.to_string())?;

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

fn remove_stale_staged(mods_dir: &Path, keep: &[String]) {
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

#[cfg(test)]
#[path = "mods_store_tests.rs"]
mod tests;
