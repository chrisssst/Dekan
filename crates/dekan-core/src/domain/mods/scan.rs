use super::*;

const MAX_DESCRIPTION_CHARS: usize = 300;

#[must_use]
pub fn is_valid_mod_dir(dir: &Path) -> bool {
    let Some(meta) = child_dir_ci(dir, "META") else {
        return false;
    };
    if !meta.join("info.json").is_file() {
        return false;
    }
    ["WAD", "RAW"].iter().any(|name| {
        child_dir_ci(dir, name).is_some_and(|content| {
            std::fs::read_dir(content)
                .map(|mut entries| entries.next().is_some())
                .unwrap_or(false)
        })
    })
}

fn child_dir_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_dir() {
        return Some(direct);
    }
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let file_name = entry.file_name();
        let matches = file_name
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name));
        (matches && entry.path().is_dir()).then(|| entry.path())
    })
}

fn is_archive(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["fantome", "zip", "modpkg"]
            .iter()
            .any(|ext| e.eq_ignore_ascii_case(ext))
    })
}

fn read_description(path: &Path, package: ModPackage) -> Option<String> {
    let file = match package {
        ModPackage::Directory => path.join("description.txt"),
        ModPackage::Archive => path.with_extension("txt"),
    };
    let text = std::fs::read_to_string(file).ok()?;
    let trimmed: String = text.trim().chars().take(MAX_DESCRIPTION_CHARS).collect();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn list_dir(dir: &Path, root: &ModRoot, category: ModCategory, id_prefix: &str) -> Vec<ModEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            warn!(dir = %dir.display(), error = %e, "Mods folder could not be read");
            return Vec::new();
        }
    };

    let mut found = Vec::new();
    let mut rejected = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            rejected += 1;
            continue;
        };
        if file_name.starts_with('.') {
            continue;
        }

        let (name, package) = if path.is_dir() {
            if category == ModCategory::Skin && file_name.parse::<u32>().is_ok() {
                continue;
            }
            if !is_valid_mod_dir(&path) {
                rejected += 1;
                continue;
            }
            (file_name, ModPackage::Directory)
        } else if path.is_file() && is_archive(&path) {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&file_name)
                .to_owned();
            (stem, ModPackage::Archive)
        } else {
            continue;
        };

        found.push(ModEntry {
            id: format!("{}:{id_prefix}/{name}", root.source.tag()),
            description: read_description(&path, package),
            name,
            category,
            source: root.source,
            path,
            package,
        });
    }

    if rejected > 0 {
        debug!(
            dir = %dir.display(),
            rejected,
            "Entries skipped: not a mod folder (META/info.json plus WAD/ or RAW/) nor an archive"
        );
    }
    found.sort_by_key(|entry| entry.name.to_lowercase());
    found
}

fn scan_skin_mods(
    root: &ModRoot,
    champion_id: ChampionId,
    belongs: &dyn Fn(&ModEntry) -> bool,
) -> Vec<ModEntry> {
    let skins_dir = root.path.join(ModCategory::Skin.folder());
    let mut found: Vec<ModEntry> = list_dir(&skins_dir, root, ModCategory::Skin, "skins")
        .into_iter()
        .filter(|entry| belongs(entry))
        .collect();

    let mut folders = vec![champion_id];
    if let Ok(entries) = std::fs::read_dir(&skins_dir) {
        folders.extend(
            entries
                .flatten()
                .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
                .filter(|id| *id / 1000 == champion_id),
        );
    }
    folders.sort_unstable();
    folders.dedup();

    for folder in folders {
        let dir = skins_dir.join(folder.to_string());
        found.extend(list_dir(
            &dir,
            root,
            ModCategory::Skin,
            &format!("skins/{folder}"),
        ));
    }
    found
}

#[must_use]
pub fn scan_catalog(
    roots: &[ModRoot],
    champion_id: Option<ChampionId>,
    belongs: &dyn Fn(&ModEntry) -> bool,
) -> ModCatalog {
    let mut catalog = ModCatalog::default();
    for root in roots {
        if let Some(champion_id) = champion_id {
            catalog
                .skin
                .extend(scan_skin_mods(root, champion_id, belongs));
        }
        for category in ModCategory::ALL.into_iter().skip(1) {
            let dir = root.path.join(category.folder());
            let listed = list_dir(&dir, root, category, category.folder());
            match category {
                ModCategory::Map => catalog.map.extend(listed),
                ModCategory::Font => catalog.font.extend(listed),
                ModCategory::Announcer => catalog.announcer.extend(listed),
                _ => catalog.others.extend(listed),
            }
        }
    }
    catalog.others.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    catalog
}
