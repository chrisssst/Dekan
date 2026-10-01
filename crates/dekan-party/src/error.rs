use thiserror::Error;

#[derive(Debug, Error)]
pub enum PartyError {
    #[error("invalid party code: {0}")]
    InvalidToken(String),
    #[error("party code expired {0} s ago")]
    ExpiredToken(u64),
    #[error("encrypted payload rejected: {0}")]
    Crypto(String),
    #[error("announcement rejected: {0}")]
    InvalidAnnouncement(String),
    #[error("relay not configured: {0}")]
    NoRelay(String),
    #[error("relay connection: {0}")]
    Relay(String),

    #[error("the party room is full")]
    RoomFull,
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}
