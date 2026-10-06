use serde::{Deserialize, Serialize};

pub type SkinId = u32;

pub type ChampionId = u32;

pub type ChromaId = u32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionMode {
    Explicit {
        skin_id: SkinId,
        chroma_id: Option<ChromaId>,
    },

    Random {
        rolled_skin_id: SkinId,
    },

    Historic {
        skin_id: SkinId,

        base_skin_id: SkinId,
    },

    CustomMod {
        mod_id: String,
        skin_id: SkinId,
        target_skin_ids: Vec<SkinId>,
    },
}

impl SelectionMode {
    #[must_use]
    pub fn effective_skin_id(&self) -> SkinId {
        match self {
            Self::Explicit { skin_id, .. }
            | Self::Random {
                rolled_skin_id: skin_id,
            }
            | Self::Historic { skin_id, .. }
            | Self::CustomMod { skin_id, .. } => *skin_id,
        }
    }

    #[must_use]
    pub fn is_base_skin(skin_id: SkinId, champion_id: ChampionId) -> bool {
        skin_id == 0 || skin_id == champion_id * 1000
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkinInfo {
    pub id: SkinId,
    pub champion_id: ChampionId,
    pub name: String,
    pub has_chromas: bool,
    pub is_owned: bool,
}
