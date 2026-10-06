use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use uuid::Uuid;
use zip::ZipArchive;

use tracing::{debug, error, info, warn};

use crate::error::PlatformError;

pub const MAX_PATH_CHARS: usize = 259;

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

pub fn validate_archive_path(entry_path: &Path) -> Result<PathBuf, PlatformError> {
    let mut clean_path = PathBuf::new();

    for comp in entry_path.components() {
        match comp {
            Component::Normal(segment) => clean_path.push(segment),
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

pub fn safe_extract_zip<R: Read + Seek>(
    reader: R,
    dest_dir: &Path,
    limits: &ExtractLimits,
) -> Result<usize, PlatformError> {
    if !dest_dir.exists() {
        std::fs::create_dir_all(dest_dir).map_err(|e| PlatformError::Io {
            context: format!(
                "failed to create destination directory '{}'",
                dest_dir.display()
            ),
            source: e,
        })?;
    }

    let mut archive = ZipArchive::new(reader)?;

    if archive.len() > limits.max_entries {
        warn!(
            dest = %dest_dir.display(),
            entries = archive.len(),
            limit = limits.max_entries,
            "Refused an archive with too many entries"
        );
        return Err(PlatformError::Security(format!(
            "archive has {} entries, exceeding maximum limit of {}",
            archive.len(),
            limits.max_entries
        )));
    }

    let mut total_extracted_bytes: u64 = 0;
    let mut extracted_count = 0;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let raw_name = file.name();

        let clean_relative = validate_archive_path(Path::new(raw_name))?;
        let target_file_path = dest_dir.join(&clean_relative);

        let path_len = target_file_path.as_os_str().encode_wide().count();
        if path_len > limits.max_path_len {
            warn!(
                entry = %clean_relative.display(),
                target = %target_file_path.display(),
                path_len,
                limit = limits.max_path_len,
                "Refused an archive entry exceeding the maximum path length"
            );
            return Err(PlatformError::Security(format!(
                "extracted path '{}' exceeds maximum allowed path length of {} characters (length: {})",
                target_file_path.display(),
                limits.max_path_len,
                path_len
            )));
        }

        if file.is_dir() {
            std::fs::create_dir_all(&target_file_path).map_err(|e| PlatformError::Io {
                context: format!(
                    "failed to create directory '{}'",
                    target_file_path.display()
                ),
                source: e,
            })?;
            continue;
        }

        if let Some(parent) = target_file_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| PlatformError::Io {
                    context: format!("failed to create parent dir '{}'", parent.display()),
                    source: e,
                })?;
            }
        }

        let mut out_file = File::create(&target_file_path).map_err(|e| PlatformError::Io {
            context: format!(
                "failed to create output file '{}'",
                target_file_path.display()
            ),
            source: e,
        })?;

        let mut buffer = [0u8; 64 * 1024];
        let mut entry_bytes: u64 = 0;

        loop {
            let bytes_read = file.read(&mut buffer).map_err(|e| PlatformError::Io {
                context: format!("read error while extracting '{}'", clean_relative.display()),
                source: e,
            })?;

            if bytes_read == 0 {
                break;
            }

            entry_bytes += bytes_read as u64;
            total_extracted_bytes += bytes_read as u64;

            if entry_bytes > limits.max_single_file_bytes {
                warn!(
                    entry = %clean_relative.display(),
                    bytes = entry_bytes,
                    limit = limits.max_single_file_bytes,
                    "Refused an archive entry that exceeded the single-file limit"
                );
                let _ = std::fs::remove_file(&target_file_path); // ignore-ok: removing a partial extraction after a refusal already logged
                return Err(PlatformError::Security(format!(
                    "file entry '{}' exceeded single file limit of {} bytes",
                    clean_relative.display(),
                    limits.max_single_file_bytes
                )));
            }

            if total_extracted_bytes > limits.max_total_bytes {
                error!(
                    dest = %dest_dir.display(),
                    entry = %clean_relative.display(),
                    extracted = total_extracted_bytes,
                    limit = limits.max_total_bytes,
                    "Aborted extraction: cumulative size limit exceeded (zip bomb)"
                );
                let _ = std::fs::remove_file(&target_file_path); // ignore-ok: removing a partial extraction after a refusal already logged
                return Err(PlatformError::Security(format!(
                    "archive exceeded total size limit of {} bytes (zip bomb detected)",
                    limits.max_total_bytes
                )));
            }

            out_file
                .write_all(&buffer[..bytes_read])
                .map_err(|e| PlatformError::Io {
                    context: format!(
                        "write error while extracting '{}'",
                        clean_relative.display()
                    ),
                    source: e,
                })?;
        }

        extracted_count += 1;
    }

    debug!(
        dest = %dest_dir.display(),
        files = extracted_count,
        bytes = total_extracted_bytes,
        "Archive extracted"
    );
    Ok(extracted_count)
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
#[path = "fs_tests.rs"]
mod tests;
