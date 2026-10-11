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

mod scan;

pub use scan::{is_valid_mod_dir, scan_catalog};

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
mod tests;
