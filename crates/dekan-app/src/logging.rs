use std::path::Path;

use anyhow::Result;
use tracing::{error, warn};
use tracing_appender::non_blocking::{ErrorCounter, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

const LOG_FILES_KEPT: usize = 7;

const DEFAULT_FILTER: &str = "info";

fn env_filter(rejected: &mut Vec<String>) -> EnvFilter {
    for var in [dekan_core::env::LOG, "RUST_LOG"] {
        if let Ok(value) = std::env::var(var) {
            let value = value.trim();
            if !value.is_empty() {
                match EnvFilter::try_new(value) {
                    Ok(filter) => return filter,
                    Err(e) => {
                        rejected.push(format!("{var}='{value}': {e}"));
                    }
                }
            }
        }
    }
    EnvFilter::new(DEFAULT_FILTER)
}

pub struct Logging {
    pub _guard: WorkerGuard,
    pub dropped: ErrorCounter,
}

pub fn init(logs_dir: &Path) -> Result<Logging> {
    std::fs::create_dir_all(logs_dir)?;

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("dekan.log")
        .max_log_files(LOG_FILES_KEPT)
        .build(logs_dir)?;
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);
    let dropped = non_blocking_file.error_counter();

    let mut rejected = Vec::new();
    let env_filter = env_filter(&mut rejected);

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_writer(non_blocking_file);

    let stdout_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true);

    let _ = tracing_subscriber::registry() // ignore-ok: try_init fails only when a subscriber is already set, which is what tests want
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init();

    for reason in rejected {
        warn!(
            reason,
            "Ignoring an invalid log filter; logging at the default level"
        );
    }

    Ok(Logging {
        _guard: guard,
        dropped,
    })
}

pub fn init_or_explain(logs_dir: &Path) -> Result<Logging> {
    init(logs_dir).inspect_err(|e| {
        let text = dekan_platform::i18n::text();
        dekan_platform::shell::message_box_warning(
            "Dekan",
            &dekan_platform::i18n::fill(
                &dekan_platform::i18n::fill(
                    text.log_folder_failed,
                    "path",
                    &logs_dir.display().to_string(),
                ),
                "error",
                &format!("{e:#}"),
            ),
        );
    })
}

pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().map_or_else(
            || "unknown".to_owned(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-text panic payload".to_owned());
        let thread = std::thread::current();
        error!(
            thread = thread.name().unwrap_or("unnamed"),
            location, message, "A thread panicked"
        );
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_filter_is_info_and_env_overrides_it() {
        unsafe {
            std::env::remove_var("DEKAN_LOG");
            std::env::remove_var("RUST_LOG");
        }
        assert_eq!(env_filter(&mut Vec::new()).to_string(), DEFAULT_FILTER);

        unsafe { std::env::set_var("DEKAN_LOG", "debug") };
        assert_eq!(env_filter(&mut Vec::new()).to_string(), "debug");

        unsafe { std::env::set_var("DEKAN_LOG", "=,,,=") };
        assert_eq!(env_filter(&mut Vec::new()).to_string(), DEFAULT_FILTER);

        unsafe { std::env::remove_var("DEKAN_LOG") };
    }

    #[test]
    fn test_logging_init_creates_dir() {
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_dir = std::env::temp_dir().join(format!("dekan_test_logs_{unique_id}"));
        let guard = init(&temp_dir);
        assert!(guard.is_ok());
        assert!(temp_dir.exists());
        let _ = std::fs::remove_dir_all(&temp_dir); // ignore-ok: test temp dir; the OS reclaims it
    }
}
