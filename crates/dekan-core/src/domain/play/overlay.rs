use serde::{Deserialize, Serialize};

use crate::mods::ModSelectionView;
use crate::selection::{ChampionId, ChromaId, SkinId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayCommand {
    Select { id: u32 },

    Clear,

    Random,

    SetMods { selection: ModSelectionView },

    OpenModsFolder,

    ImportMod { category: crate::mods::ModCategory },

    ChromaPreview { id: u32 },

    FocusChampion { id: u32 },

    TogglePreset,

    SetProfile { name: String },

    NewProfile,

    DeleteProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOrigin {
    Historic,
    Random,
    Preset,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetsView {
    pub profiles: Vec<String>,
    pub active: usize,
    pub preset_entry: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayTarget {
    pub champion_id: ChampionId,

    pub skin_id: SkinId,

    pub chroma_id: Option<ChromaId>,
}

impl OverlayTarget {
    #[must_use]
    pub fn package_entry_id(&self) -> u32 {
        self.chroma_id.unwrap_or(self.skin_id)
    }

    #[must_use]
    pub fn matches_champion(&self, champion_id: ChampionId) -> bool {
        self.champion_id == champion_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogChroma {
    pub id: u32,
    pub name: String,

    pub color: Option<String>,

    pub form: bool,

    pub preview_path: Option<String>,

    pub has_preview: bool,
}

impl CatalogChroma {
    #[must_use]
    pub fn with_preview(mut self, path: Option<&str>) -> Self {
        self.preview_path = path.filter(|p| p.starts_with('/')).map(str::to_owned);
        self.has_preview = self.preview_path.is_some();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSkin {
    pub id: u32,
    pub name: String,

    pub name_unknown: bool,
    pub chromas: Vec<CatalogChroma>,

    pub tile: Option<std::sync::Arc<[u8]>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub champion_id: u32,
    pub champion_name: String,

    pub alias: Option<String>,
    pub skins: Vec<CatalogSkin>,

    pub locale: Option<String>,

    pub quote: Option<String>,

    pub mods: ModsPanel,

    pub notice: Option<CatalogNotice>,

    pub classic: bool,

    pub lobby: Vec<LobbyChampion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyChampion {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogNotice {
    ToolsMissing,

    LobbyWaiting,

    LobbyChampions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModsPanel {
    pub available: crate::mods::ModCatalog,
    pub selection: crate::mods::ModSelectionView,
}

impl Catalog {
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.skins.iter().map(|s| 1 + s.chromas.len()).sum()
    }

    pub fn roll_random(&self, mut pick: impl FnMut(usize) -> usize) -> Option<OverlayTarget> {
        let candidates: Vec<&CatalogSkin> = self
            .skins
            .iter()
            .filter(|skin| !crate::selection::is_base_skin(skin.id, self.champion_id))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let skin = candidates[pick(candidates.len()) % candidates.len()];
        let options = 1 + skin.chromas.len();
        let entry = match pick(options) % options {
            0 => skin.id,
            n => skin.chromas[n - 1].id,
        };
        self.resolve_target(entry)
    }

    #[must_use]
    pub fn resolve_target(&self, entry_id: u32) -> Option<OverlayTarget> {
        for skin in &self.skins {
            if skin.id == entry_id {
                return Some(OverlayTarget {
                    champion_id: self.champion_id,
                    skin_id: skin.id,
                    chroma_id: None,
                });
            }
            if let Some(chroma) = skin.chromas.iter().find(|c| c.id == entry_id) {
                return Some(OverlayTarget {
                    champion_id: self.champion_id,
                    skin_id: skin.id,
                    chroma_id: Some(chroma.id),
                });
            }
        }
        None
    }

    #[must_use]
    pub fn chroma_preview_paths(&self) -> Vec<(u32, String)> {
        self.skins
            .iter()
            .flat_map(|s| s.chromas.iter())
            .filter_map(|c| c.preview_path.clone().map(|path| (c.id, path)))
            .collect()
    }

    #[must_use]
    pub fn chroma_preview_path(&self, chroma_id: u32) -> Option<&str> {
        self.skins
            .iter()
            .flat_map(|s| s.chromas.iter())
            .find(|c| c.id == chroma_id)
            .and_then(|c| c.preview_path.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chroma_is_the_entry_that_gets_installed() {
        let skin = OverlayTarget {
            champion_id: 238,
            skin_id: 238_001,
            chroma_id: None,
        };
        assert_eq!(skin.package_entry_id(), 238_001);

        let chroma = OverlayTarget {
            champion_id: 238,
            skin_id: 238_001,
            chroma_id: Some(238_015),
        };
        assert_eq!(
            chroma.package_entry_id(),
            238_015,
            "a chroma has its own package; injecting the parent skin would show the wrong colours"
        );
    }

    #[test]
    fn a_target_never_crosses_champions() {
        let target = OverlayTarget {
            champion_id: 238,
            skin_id: 238_001,
            chroma_id: None,
        };
        assert!(target.matches_champion(238));
        assert!(
            !target.matches_champion(103),
            "a Zed pick must not be injected for Ahri after a bench swap"
        );
    }
}
