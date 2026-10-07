use thiserror::Error;

#[derive(Debug, Error)]
pub enum InjectError {
    #[error("the LTK injector at {path} is not trusted: {reason}")]
    UntrustedInjector { path: String, reason: String },
    #[error("overlay build failed: {0}")]
    Overlay(String),

    #[error("cancelled")]
    Cancelled,
    #[error("the game was already loading for {age_ms}ms; it is not hooked mid-load")]
    GameAlreadyLoading { age_ms: u64 },
    #[error("hook not confirmed after {timeout_ms}ms")]
    HookUnconfirmed { timeout_ms: u64 },
    #[error("process error: {0}")]
    Process(String),
    #[error("subprocess execution timed out after {timeout_secs}s: {command}")]
    SubprocessTimeout { command: String, timeout_secs: u64 },
    #[error("platform error: {0}")]
    Platform(#[from] dekan_platform::error::PlatformError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}
