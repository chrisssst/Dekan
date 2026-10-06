use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::selection::ChampionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModCategory {
    Skin,
    Map,
    Font,
    Announcer,
    Ui,
    Voiceover,
    LoadingScreen,
    Vfx,
    Sfx,
    Other,
}

impl ModCategory {
    pub const ALL: [ModCategory; 10] = [
        Self::Skin,
        Self::Map,
        Self::Font,
        Self::Announcer,
        Self::Ui,
        Self::Voiceover,
        Self::LoadingScreen,
        Self::Vfx,
        Self::Sfx,
        Self::Other,
    ];

    #[must_use]
    pub fn folder(self) -> &'static str {
        match self {
            Self::Skin => "skins",
            Self::Map => "maps",
            Self::Font => "fonts",
            Self::Announcer => "announcers",
            Self::Ui => "ui",
            Self::Voiceover => "voiceover",
            Self::LoadingScreen => "loading_screen",
            Self::Vfx => "vfx",
            Self::Sfx => "sfx",
            Self::Other => "others",
        }
    }

    #[must_use]
    pub fn is_single_choice(self) -> bool {
        matches!(self, Self::Skin | Self::Map | Self::Font | Self::Announcer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModSource {
    Dekan,
}

impl ModSource {
    fn tag(self) -> &'static str {
        match self {
            Self::Dekan => "dekan",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModPackage {
    Directory,
    Archive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModEntry {
    pub id: String,
    pub name: String,
    pub category: ModCategory,
    pub source: ModSource,
    #[serde(skip)]
    pub path: PathBuf,
    pub package: ModPackage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModRoot {
    pub path: PathBuf,
    pub source: ModSource,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModCatalog {
    pub skin: Vec<ModEntry>,
    pub map: Vec<ModEntry>,
    pub font: Vec<ModEntry>,
    pub announcer: Vec<ModEntry>,
    pub others: Vec<ModEntry>,
}

impl ModCatalog {
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&ModEntry> {
        self.skin
            .iter()
            .chain(&self.map)
            .chain(&self.font)
            .chain(&self.announcer)
            .chain(&self.others)
            .find(|entry| entry.id == id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.skin.len()
            + self.map.len()
            + self.font.len()
            + self.announcer.len()
            + self.others.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

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
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("fantome") || e.eq_ignore_ascii_case("zip"))
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModSelection {
    pub skin: BTreeMap<ChampionId, String>,
    pub map: Option<String>,
    pub font: Option<String>,
    pub announcer: Option<String>,

    pub others: Vec<String>,
}

impl ModSelection {
    #[must_use]
    pub fn ordered_ids(&self, champion_id: Option<ChampionId>) -> Vec<&str> {
        let mut ids: Vec<&str> = Vec::new();
        if let Some(skin) = champion_id.and_then(|c| self.skin.get(&c)) {
            ids.push(skin);
        }
        ids.extend(self.map.as_deref());
        ids.extend(self.font.as_deref());
        ids.extend(self.announcer.as_deref());
        ids.extend(self.others.iter().map(String::as_str));
        ids
    }

    #[must_use]
    pub fn fingerprint(&self, champion_id: Option<ChampionId>) -> u64 {
        let ids = self.ordered_ids(champion_id);
        if ids.is_empty() {
            return 0;
        }
        let mut hash = FNV_OFFSET;
        for id in ids {
            hash = fnv1a64(hash, id.as_bytes());
            hash = fnv1a64(hash, &[0]);
        }

        hash.max(1)
    }

    #[must_use]
    pub fn view(&self, champion_id: Option<ChampionId>) -> ModSelectionView {
        ModSelectionView {
            skin: champion_id.and_then(|c| self.skin.get(&c).cloned()),
            map: self.map.clone(),
            font: self.font.clone(),
            announcer: self.announcer.clone(),
            others: self.others.clone(),
        }
    }

    pub fn prune(&mut self, catalog: &ModCatalog, champion_id: Option<ChampionId>) -> Vec<String> {
        let mut dropped = Vec::new();
        let mut keep = |id: &mut Option<String>| {
            if id.as_deref().is_some_and(|v| catalog.find(v).is_none()) {
                dropped.extend(id.take());
            }
        };
        keep(&mut self.map);
        keep(&mut self.font);
        keep(&mut self.announcer);
        if let Some(champion_id) = champion_id {
            let mut skin = self.skin.get(&champion_id).cloned();
            keep(&mut skin);
            if skin.is_none() {
                self.skin.remove(&champion_id);
            }
        }
        self.others.retain(|id| {
            let present = catalog.find(id).is_some();
            if !present {
                dropped.push(id.clone());
            }
            present
        });
        dropped
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModSelectionView {
    pub skin: Option<String>,
    pub map: Option<String>,
    pub font: Option<String>,
    pub announcer: Option<String>,
    pub others: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedMod {
    pub id: String,
    pub reason: &'static str,
}

impl ModCatalog {
    pub fn apply_request(
        &self,
        current: &ModSelection,
        champion_id: Option<ChampionId>,
        request: &ModSelectionView,
    ) -> (ModSelection, Vec<RejectedMod>) {
        let mut next = current.clone();
        let mut rejected = Vec::new();

        let mut check =
            |id: &Option<String>, list: &[ModEntry], slot: &'static str| -> Option<String> {
                let id = id.as_ref()?;
                if list.iter().any(|e| &e.id == id) {
                    Some(id.clone())
                } else {
                    rejected.push(RejectedMod {
                        id: id.clone(),
                        reason: slot,
                    });
                    None
                }
            };

        next.map = check(&request.map, &self.map, "not a listed map mod");
        next.font = check(&request.font, &self.font, "not a listed font mod");
        next.announcer = check(
            &request.announcer,
            &self.announcer,
            "not a listed announcer mod",
        );

        match champion_id {
            Some(champion_id) => {
                match check(&request.skin, &self.skin, "not a skin mod of this champion") {
                    Some(id) => {
                        next.skin.insert(champion_id, id);
                    }
                    None => {
                        next.skin.remove(&champion_id);
                    }
                }
            }
            None => {
                if let Some(id) = &request.skin {
                    rejected.push(RejectedMod {
                        id: id.clone(),
                        reason: "no champion to attach a skin mod to",
                    });
                }
            }
        }

        let mut others: Vec<String> = Vec::new();
        for id in &request.others {
            if self.others.iter().any(|e| &e.id == id) {
                others.push(id.clone());
            } else {
                rejected.push(RejectedMod {
                    id: id.clone(),
                    reason: "not a listed multi-choice mod",
                });
            }
        }
        others.sort();
        others.dedup();
        next.others = others;

        (next, rejected)
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a64(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[must_use]
pub fn staged_name(id: &str, stamp: &str) -> String {
    let hash = fnv1a64(fnv1a64(FNV_OFFSET, id.as_bytes()), stamp.as_bytes());
    format!("cm_{hash:016x}")
}

pub const STAGED_PREFIX: &str = "cm_";

#[cfg(test)]
#[path = "mods_tests.rs"]
mod tests;
