use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use uuid::Uuid;

use tracing::{debug, error, info, warn};

use crate::error::PlatformError;

mod archive;

pub use archive::{ExtractLimits, MAX_PATH_CHARS, safe_extract_zip, validate_archive_path};

pub fn atomic_write(target_path: &Path, content: &[u8], sync: bool) -> Result<(), PlatformError> {
    let parent = target_path.parent().ok_or_else(|| {
        PlatformError::Path(format!(
            "target path '{}' has no parent directory",
            target_path.display()
        ))
    })?;

    if !parent.exists() {
        std::fs::create_dir_all(parent).map_err(|e| PlatformError::Io {
            context: format!("failed to create directory '{}'", parent.display()),
            source: e,
        })?;
    }

    let tmp_filename = format!(
        ".tmp-{}-{}",
        Uuid::new_v4(),
        target_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("file")
    );
    let tmp_path = parent.join(tmp_filename);

    let write_result = (|| -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;

        file.write_all(content)?;
        file.flush()?;

        if sync {
            file.sync_all()?;
        }
        Ok(())
    })();

    if let Err(e) = write_result {
        warn!(
            tmp = %tmp_path.display(),
            target = %target_path.display(),
            bytes = content.len(),
            error = %e,
            "Could not write the temporary file for an atomic write"
        );
        let _ = std::fs::remove_file(&tmp_path); // ignore-ok: cleaning up our own temp file after a failure already reported
        return Err(PlatformError::Io {
            context: format!("failed to write temporary file '{}'", tmp_path.display()),
            source: e,
        });
    }

    const MAX_RETRIES: usize = 5;
    let mut delay = Duration::from_millis(10);

    for attempt in 1..=MAX_RETRIES {
        match std::fs::rename(&tmp_path, target_path) {
            Ok(()) => {
                if attempt > 1 {
                    info!(
                        target = %target_path.display(),
                        attempts = attempt,
                        "Atomic write succeeded after retrying a locked target"
                    );
                }
                return Ok(());
            }
            Err(e) if attempt < MAX_RETRIES => {
                let raw_code = e.raw_os_error().unwrap_or(0);

                if raw_code == 5
                    || raw_code == 32
                    || e.kind() == std::io::ErrorKind::PermissionDenied
                {
                    warn!(
                        target = %target_path.display(),
                        attempt,
                        os_error = raw_code,
                        delay_ms = delay.as_millis(),
                        "Target file is locked (likely an antivirus scan); retrying"
                    );
                    sleep(delay);
                    delay *= 2;
                    continue;
                }
                warn!(
                    target = %target_path.display(),
                    os_error = raw_code,
                    error = %e,
                    "Atomic write failed for a reason retrying cannot fix"
                );
                let _ = std::fs::remove_file(&tmp_path); // ignore-ok: cleaning up our own temp file after a failure already reported
                return Err(PlatformError::Io {
                    context: format!(
                        "failed to rename '{}' to '{}'",
                        tmp_path.display(),
                        target_path.display()
                    ),
                    source: e,
                });
            }
            Err(e) => {
                error!(
                    target = %target_path.display(),
                    attempts = MAX_RETRIES,
                    os_error = e.raw_os_error().unwrap_or(0),
                    error = %e,
                    "Atomic write gave up after exhausting every retry"
                );
                let _ = std::fs::remove_file(&tmp_path); // ignore-ok: cleaning up our own temp file after a failure already reported
                return Err(PlatformError::Io {
                    context: format!(
                        "failed to rename '{}' to '{}' after {MAX_RETRIES} attempts",
                        tmp_path.display(),
                        target_path.display()
                    ),
                    source: e,
                });
            }
        }
    }

    let _ = std::fs::remove_file(&tmp_path); // ignore-ok: cleaning up our own temp file after a failure already reported
    Ok(())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MirrorStats {
    pub linked: usize,

    pub copied: usize,

    pub skipped: usize,
}

pub fn mirror_tree(src: &Path, dst: &Path) -> Result<MirrorStats, PlatformError> {
    if dst.exists() {
        return Err(PlatformError::Io {
            context: format!("mirror destination '{}' already exists", dst.display()),
            source: std::io::Error::from(std::io::ErrorKind::AlreadyExists),
        });
    }

    let mut stats = MirrorStats::default();
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((from_dir, to_dir)) = stack.pop() {
        std::fs::create_dir_all(&to_dir).map_err(|e| PlatformError::Io {
            context: format!("failed to create mirror directory '{}'", to_dir.display()),
            source: e,
        })?;

        let entries = std::fs::read_dir(&from_dir).map_err(|e| PlatformError::Io {
            context: format!("failed to read '{}'", from_dir.display()),
            source: e,
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| PlatformError::Io {
                context: format!("failed to list '{}'", from_dir.display()),
                source: e,
            })?;
            let from = entry.path();
            let to = to_dir.join(entry.file_name());

            let path_len = to.as_os_str().encode_wide().count();
            if path_len > MAX_PATH_CHARS {
                warn!(
                    from = %from.display(),
                    to = %to.display(),
                    path_len,
                    limit = MAX_PATH_CHARS,
                    "Mirror target path exceeds Windows MAX_PATH"
                );
                return Err(PlatformError::Security(format!(
                    "mirror target path exceeds Windows MAX_PATH of {MAX_PATH_CHARS} characters: '{}' (length: {path_len})",
                    to.display()
                )));
            }

            let kind = std::fs::symlink_metadata(&from)
                .map_err(|e| PlatformError::Io {
                    context: format!("failed to stat '{}'", from.display()),
                    source: e,
                })?
                .file_type();

            if kind.is_dir() {
                stack.push((from, to));
            } else if kind.is_file() {
                if std::fs::hard_link(&from, &to).is_ok() {
                    stats.linked += 1;
                } else {
                    std::fs::copy(&from, &to).map_err(|e| PlatformError::Io {
                        context: format!(
                            "failed to copy '{}' to '{}'",
                            from.display(),
                            to.display()
                        ),
                        source: e,
                    })?;
                    stats.copied += 1;
                }
            } else {
                stats.skipped += 1;
            }
        }
    }

    if stats.copied > 0 || stats.skipped > 0 {
        warn!(
            src = %src.display(),
            linked = stats.linked,
            copied = stats.copied,
            skipped = stats.skipped,
            "Mirror could not hard-link everything"
        );
    } else {
        debug!(src = %src.display(), linked = stats.linked, "Mirror built with hard links");
    }
    Ok(stats)
}

pub fn get_disk_free_space(path: &Path) -> Result<u64, PlatformError> {
    let mut path_buf = path.to_path_buf();
    if !path_buf.exists() {
        if let Some(parent) = path.parent() {
            path_buf = parent.to_path_buf();
        }
    }

    let wide_path: Vec<u16> = path_buf
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut free_bytes: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut total_free: u64 = 0;

    unsafe {
        windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            windows::core::PCWSTR(wide_path.as_ptr()),
            Some(&mut free_bytes),
            Some(&mut total_bytes),
            Some(&mut total_free),
        )
    }
    .map_err(|e| PlatformError::Io {
        context: format!("failed to get disk free space for '{}'", path.display()),
        source: std::io::Error::from_raw_os_error(e.code().0),
    })?;

    Ok(free_bytes)
}

#[cfg(test)]
mod tests;
