use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;
use tracing::{debug, error, warn};
use zip::ZipArchive;

use super::path::{ExtractLimits, link_on_the_way, validate_archive_path};
use crate::error::PlatformError;
use crate::os::storage::fs::path_units;

#[cfg(windows)]
fn open_pinned_root(dest_dir: &Path, canonical_dest: &Path) -> Result<File, PlatformError> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_LIST_DIRECTORY: u32 = 0x0000_0001;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let root_file = std::fs::OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(dest_dir)
        .map_err(|source| PlatformError::Io {
            context: format!(
                "failed to open pinned destination directory '{}'",
                dest_dir.display()
            ),
            source,
        })?;
    verify_handle_within_root(&root_file, canonical_dest)?;
    Ok(root_file)
}

#[cfg(not(windows))]
fn open_pinned_root(dest_dir: &Path, _canonical_dest: &Path) -> Result<File, PlatformError> {
    File::open(dest_dir).map_err(|source| PlatformError::Io {
        context: format!(
            "failed to open destination directory '{}'",
            dest_dir.display()
        ),
        source,
    })
}

#[cfg(windows)]
fn verify_handle_within_root(file: &File, canonical_dest: &Path) -> Result<(), PlatformError> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::PathBuf;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FILE_NAME_NORMALIZED, GETFINALPATHNAMEBYHANDLE_FLAGS, GetFinalPathNameByHandleW,
        VOLUME_NAME_DOS,
    };

    let handle = HANDLE(file.as_raw_handle() as _);
    let mut buffer = [0u16; 1024];
    let flags = GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0);
    let len = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, flags) };
    if len == 0 || len as usize >= buffer.len() {
        return Err(PlatformError::Security(
            "could not resolve final path of created file to verify reparse point safety".into(),
        ));
    }
    let final_os_str = std::ffi::OsString::from_wide(&buffer[..len as usize]);
    let final_path = PathBuf::from(final_os_str);

    if !final_path.starts_with(canonical_dest) {
        return Err(PlatformError::Security(format!(
            "created file '{}' escaped destination root '{}' via reparse point (TOCTOU violation)",
            final_path.display(),
            canonical_dest.display()
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn verify_handle_within_root(_file: &File, _canonical_dest: &Path) -> Result<(), PlatformError> {
    Ok(())
}

#[cfg(windows)]
fn discard_partial(file: File) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
    };

    let handle = HANDLE(file.as_raw_handle() as _);
    let mut disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let result = unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfo,
            &mut disposition as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };

    if let Err(e) = result {
        warn!(
            error = %e,
            "Could not mark partial file for deletion by handle; dropping handle without unsafe path removal fallback"
        );
    }
    drop(file);
}

#[cfg(not(windows))]
fn discard_partial(file: File) {
    drop(file);
}

pub fn safe_extract_zip<R: Read + Seek>(
    reader: R,
    dest_dir: &Path,
    limits: &ExtractLimits,
) -> Result<usize, PlatformError> {
    if let Some(link) = link_on_the_way(dest_dir, Path::new(""))? {
        return Err(PlatformError::Security(format!(
            "archive destination '{}' traverses the link '{}'",
            dest_dir.display(),
            link.display()
        )));
    }

    if !dest_dir.exists() {
        std::fs::create_dir_all(dest_dir).map_err(|e| PlatformError::Io {
            context: format!(
                "failed to create destination directory '{}'",
                dest_dir.display()
            ),
            source: e,
        })?;
    }

    let canonical_dest = dest_dir.canonicalize().map_err(|e| PlatformError::Io {
        context: format!(
            "failed to canonicalize destination directory '{}'",
            dest_dir.display()
        ),
        source: e,
    })?;
    let pinned_root = open_pinned_root(dest_dir, &canonical_dest)?;

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
        let raw_name = file.name()?.into_owned();

        let clean_relative = validate_archive_path(Path::new(&raw_name))?;
        let target_file_path = dest_dir.join(&clean_relative);

        let path_len = path_units(&target_file_path);
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

        if file.is_symlink() {
            warn!(
                entry = %clean_relative.display(),
                "Refused an archive entry that is a symbolic link"
            );
            return Err(PlatformError::Security(format!(
                "archive entry is a symbolic link: '{}'",
                clean_relative.display()
            )));
        }

        if let Some(link) = link_on_the_way(dest_dir, &clean_relative)? {
            warn!(
                entry = %clean_relative.display(),
                link = %link.display(),
                "Refused an archive entry that would be written through a link or junction"
            );
            return Err(PlatformError::Security(format!(
                "archive entry '{}' would be written through the link '{}'",
                clean_relative.display(),
                link.display()
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

        let mut open_opts = std::fs::OpenOptions::new();
        open_opts.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_GENERIC_WRITE: u32 = 0x4000_0000;
            const DELETE_ACCESS: u32 = 0x0001_0000;
            open_opts
                .access_mode(FILE_GENERIC_WRITE | DELETE_ACCESS)
                .share_mode(0);
        }

        let out_file = open_opts
            .open(&target_file_path)
            .map_err(|e| PlatformError::Io {
                context: format!(
                    "failed to create output file '{}'",
                    target_file_path.display()
                ),
                source: e,
            })?;

        if let Err(e) = verify_handle_within_root(&pinned_root, &canonical_dest)
            .and_then(|()| verify_handle_within_root(&out_file, &canonical_dest))
        {
            discard_partial(out_file);
            return Err(e);
        }
        let mut out_file = out_file;

        let mut buffer = [0u8; 64 * 1024];
        let mut entry_bytes: u64 = 0;
        let mut content = (&mut file).take(declared_size);

        loop {
            let bytes_read = match content.read(&mut buffer) {
                Ok(read) => read,
                Err(e) => {
                    discard_partial(out_file);
                    return Err(PlatformError::Io {
                        context: format!(
                            "read error while extracting '{}'",
                            clean_relative.display()
                        ),
                        source: e,
                    });
                }
            };

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
                discard_partial(out_file);
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
                discard_partial(out_file);
                return Err(PlatformError::Security(format!(
                    "archive exceeded total size limit of {} bytes (zip bomb detected)",
                    limits.max_total_bytes
                )));
            }

            if let Err(e) = verify_handle_within_root(&pinned_root, &canonical_dest)
                .and_then(|()| verify_handle_within_root(&out_file, &canonical_dest))
            {
                discard_partial(out_file);
                return Err(e);
            }

            if let Err(e) = out_file.write_all(&buffer[..bytes_read]) {
                discard_partial(out_file);
                return Err(PlatformError::Io {
                    context: format!(
                        "write error while extracting '{}'",
                        clean_relative.display()
                    ),
                    source: e,
                });
            }
        }

        if entry_bytes != declared_size {
            discard_partial(out_file);
            return Err(PlatformError::Io {
                context: format!(
                    "'{}' is truncated: {entry_bytes} of {declared_size} bytes",
                    clean_relative.display()
                ),
                source: std::io::ErrorKind::UnexpectedEof.into(),
            });
        }

        if let Err(e) = verify_handle_within_root(&out_file, &canonical_dest) {
            discard_partial(out_file);
            return Err(e);
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
