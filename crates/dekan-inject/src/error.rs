use thiserror::Error;

#[derive(Debug, Error)]
pub enum InjectError {
    #[error("DLL hash mismatch: expected {expected}, got {actual}")]
    DllHashMismatch { expected: String, actual: String },
    #[error("suspension failed: {0}")]
    Suspend(String),
    #[error("overlay build failed: {0}")]
    Overlay(String),

    #[error("cancelled")]
    Cancelled,
    #[error("hook not confirmed after {timeout_ms}ms")]
    HookUnconfirmed { timeout_ms: u64 },
    #[error("process error: {0}")]
    Process(String),
    #[error(
        "insufficient disk space: required {required_bytes} bytes, available {available_bytes} bytes on '{path}'"
    )]
    InsufficientDiskSpace {
        path: String,
        required_bytes: u64,
        available_bytes: u64,
    },
    #[error("subprocess execution timed out after {timeout_secs}s: {command}")]
    SubprocessTimeout { command: String, timeout_secs: u64 },
    #[error("subprocess failed with exit code {exit_code}: {details}")]
    SubprocessFailed { exit_code: i32, details: String },
    #[error("platform error: {0}")]
    Platform(#[from] dekan_platform::error::PlatformError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}
