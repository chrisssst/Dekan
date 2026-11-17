use std::path::{Component, Path, PathBuf};
use tracing::warn;

use crate::error::PlatformError;
use crate::os::storage::fs::MAX_PATH_CHARS;

#[derive(Debug, Clone)]
pub struct ExtractLimits {
    pub max_total_bytes: u64,
    pub max_single_file_bytes: u64,
    pub max_entries: usize,
    pub max_path_len: usize,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_single_file_bytes: 500 * 1024 * 1024,
            max_entries: 10_000,
            max_path_len: MAX_PATH_CHARS,
        }
    }
}

fn is_portable_segment(segment: &std::ffi::OsStr) -> bool {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let Some(name) = segment.to_str() else {
        return false;
    };
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    !name.is_empty()
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*'))
        && !RESERVED
            .iter()
            .any(|device| stem.eq_ignore_ascii_case(device))
}

pub fn validate_archive_path(entry_path: &Path) -> Result<PathBuf, PlatformError> {
    let mut clean_path = PathBuf::new();

    for comp in entry_path.components() {
        match comp {
            Component::Normal(segment) if is_portable_segment(segment) => clean_path.push(segment),
            Component::Normal(_) => {
                warn!(
                    entry = %entry_path.display(),
                    "Refused an archive entry with a name Windows treats specially"
                );
                return Err(PlatformError::Security(format!(
                    "archive entry has a reserved or unsafe name: '{}'",
                    entry_path.display()
                )));
            }
            Component::ParentDir => {
                warn!(
                    entry = %entry_path.display(),
                    "Refused an archive entry with parent-directory traversal"
                );
                return Err(PlatformError::Security(format!(
                    "archive entry contains parent directory traversal ('..'): '{}'",
                    entry_path.display()
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                warn!(
                    entry = %entry_path.display(),
                    "Refused an archive entry with an absolute or drive-qualified path"
                );
                return Err(PlatformError::Security(format!(
                    "archive entry specifies absolute or drive path: '{}'",
                    entry_path.display()
                )));
            }
            Component::CurDir => {}
        }
    }

    if clean_path.as_os_str().is_empty() {
        return Err(PlatformError::Security(
            "archive entry resolves to empty path".into(),
        ));
    }

    Ok(clean_path)
}

pub(super) fn link_on_the_way(
    dest_dir: &Path,
    relative: &Path,
) -> Result<Option<PathBuf>, PlatformError> {
    let target = dest_dir.join(relative);
    let target = if target.is_absolute() {
        target
    } else {
        std::env::current_dir()
            .map_err(|source| PlatformError::Io {
                context: "failed to resolve archive destination".into(),
                source,
            })?
            .join(target)
    };
    let mut current = PathBuf::new();
    for component in target.components() {
        current.push(component);
        if matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::CurDir
        ) {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Ok(Some(current)),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!(
                    path = %current.display(),
                    error = %e,
                    "Refused an archive entry whose path could not be checked for links"
                );
                return Err(PlatformError::Io {
                    context: format!("failed to check '{}' for links", current.display()),
                    source: e,
                });
            }
        }
    }
    Ok(None)
}
