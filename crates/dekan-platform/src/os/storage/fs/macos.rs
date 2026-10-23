use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::error::PlatformError;

pub const MAX_PATH_CHARS: usize = 1023;

pub(super) fn path_units(path: &Path) -> usize {
    path.as_os_str().len()
}

pub fn get_disk_free_space(path: &Path) -> Result<u64, PlatformError> {
    let target = if path.exists() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let failure = |source| PlatformError::Io {
        context: format!("failed to get disk free space for '{}'", path.display()),
        source,
    };
    let c_path = CString::new(target.as_os_str().as_bytes())
        .map_err(|e| failure(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    if unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(failure(std::io::Error::last_os_error()));
    }
    let stats = unsafe { stats.assume_init() };
    Ok(u64::from(stats.f_bavail).saturating_mul(stats.f_frsize))
}
