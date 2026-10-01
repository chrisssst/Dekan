use thiserror::Error;

#[derive(Debug, Error)]
pub enum LcuError {
    #[error("lockfile not found")]
    LockfileNotFound,

    #[error("invalid lockfile format: {0}")]
    InvalidLockfile(String),

    #[error("local player not found in team session")]
    LocalPlayerNotFound,

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("request timed out")]
    Timeout,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
