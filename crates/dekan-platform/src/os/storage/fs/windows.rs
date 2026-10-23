use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use crate::error::PlatformError;

pub const MAX_PATH_CHARS: usize = 259;

pub(super) fn path_units(path: &Path) -> usize {
    path.as_os_str().encode_wide().count()
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
