use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("Win32 error: {0}")]
    Win32(#[from] windows::core::Error),

    #[error("process not found: {name}")]
    ProcessNotFound { name: String },

    #[error("another instance is already running (PID: {pid:?})")]
    AlreadyRunning { pid: Option<u32> },

    #[error("window error: {0}")]
    Window(String),

    #[error("path error: {0}")]
    Path(String),

    #[error("security violation: {0}")]
    Security(String),

    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
}
