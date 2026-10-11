use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use zip::ZipArchive;

use tracing::{debug, error, warn};

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
        let declared_size = file.size();
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
        let mut content = (&mut file).take(declared_size);

        loop {
            let bytes_read = content.read(&mut buffer).map_err(|e| PlatformError::Io {
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

        if entry_bytes != declared_size {
            let _ = std::fs::remove_file(&target_file_path); // ignore-ok: removing a truncated extraction; the refusal is what gets reported
            return Err(PlatformError::Io {
                context: format!(
                    "'{}' is truncated: {entry_bytes} of {declared_size} bytes",
                    clean_relative.display()
                ),
                source: std::io::ErrorKind::UnexpectedEof.into(),
            });
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
