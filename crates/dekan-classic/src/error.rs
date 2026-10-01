use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClassicError {
    #[error("champion not found in JADE tree: {alias}")]
    ChampionNotFound { alias: String },
    #[error("skin {skin_id} not found for champion {champion_id}")]
    SkinNotFound { champion_id: u32, skin_id: u32 },
    #[error("unsafe champion alias '{0}' (only [A-Za-z0-9_] is accepted)")]
    InvalidAlias(String),
    #[error("skin bin could not be rebuilt: {0}")]
    Bin(String),
    #[error("WAD error: {0}")]
    Wad(#[from] dekan_wad::error::WadError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}
