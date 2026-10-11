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

pub(crate) const CUSTOM_MOD_LIMITS: ExtractLimits = ExtractLimits {
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
                    ["fantome", "zip", "modpkg"]
                        .iter()
                        .any(|ext| e.eq_ignore_ascii_case(ext))
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
        ModPackage::Archive if is_modpkg_file(&entry.path) => match read_modpkg(&entry.path) {
            Ok(package) => package.wad_names(),
            Err(e) => {
                debug!(mod_path = %entry.path.display(), error = %e, "Mod package could not be listed");
                BTreeSet::new()
            }
        },
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

mod import;
mod stage;

pub use import::{ImportRefusal, import_archive};
pub use stage::stage_selected;
pub(crate) use stage::{STAGING_HEADROOM, unpack_modpkg, unpacked_size};
use stage::{is_modpkg_file, read_modpkg};

#[cfg(test)]
pub(crate) mod tests;
