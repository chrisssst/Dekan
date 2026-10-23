use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::debug;

use crate::error::PlatformError;

#[must_use = "the thread leaves background mode when this is dropped"]
pub struct BackgroundThread(());

impl BackgroundThread {
    pub fn enter() -> Option<Self> {
        if unsafe { libc::setpriority(libc::PRIO_DARWIN_THREAD, 0, libc::PRIO_DARWIN_BG) } == 0 {
            Some(Self(()))
        } else {
            debug!(
                error = %std::io::Error::last_os_error(),
                "Background thread mode unavailable; the work runs at normal priority"
            );
            None
        }
    }
}

impl Drop for BackgroundThread {
    fn drop(&mut self) {
        let _ = unsafe { libc::setpriority(libc::PRIO_DARWIN_THREAD, 0, 0) }; // ignore-ok: the thread is ending its background work; a failure only leaves it at low priority
    }
}

pub struct ProcessFinder;

impl ProcessFinder {
    #[must_use]
    pub fn process_age(pid: u32) -> Option<Duration> {
        let info = bsd_info(pid)?;
        let started = UNIX_EPOCH
            + Duration::from_secs(info.pbi_start_tvsec)
            + Duration::from_micros(info.pbi_start_tvusec);
        SystemTime::now().duration_since(started).ok()
    }

    pub fn get_process_path(pid: u32) -> Result<Option<PathBuf>, PlatformError> {
        let Ok(pid) = libc::c_int::try_from(pid) else {
            return Ok(None);
        };
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let written =
            unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if written <= 0 {
            debug!(pid, error = %std::io::Error::last_os_error(), "Process image path unavailable");
            return Ok(None);
        }
        buffer.truncate(written as usize);
        Ok(Some(PathBuf::from(OsStr::from_bytes(&buffer))))
    }

    pub fn find_process_path(exe_name: &str) -> Result<Option<PathBuf>, PlatformError> {
        match Self::find_process_by_name(exe_name)? {
            Some(pid) => Self::get_process_path(pid),
            None => Ok(None),
        }
    }

    pub fn find_process_by_name(exe_name: &str) -> Result<Option<u32>, PlatformError> {
        Self::find_any_process(&[exe_name])
    }

    pub fn find_any_process(exe_names: &[&str]) -> Result<Option<u32>, PlatformError> {
        for pid in all_pids()? {
            let Ok(Some(path)) = Self::get_process_path(pid) else {
                continue;
            };
            let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            if exe_names.iter().any(|exe| name.eq_ignore_ascii_case(exe)) {
                return Ok(Some(pid));
            }
        }
        Ok(None)
    }

    pub fn find_first_thread_id(pid: u32) -> Result<Option<u32>, PlatformError> {
        if bsd_info(pid).is_some() {
            return Ok(Some(pid));
        }
        debug!(pid, "No process found for this PID; the process is gone");
        Ok(None)
    }
}

fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let size = libc::c_int::try_from(size_of::<libc::proc_bsdinfo>()).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    (written == size).then(|| unsafe { info.assume_init() })
}

fn all_pids() -> Result<Vec<u32>, PlatformError> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(PlatformError::Io {
            context: "listing processes".into(),
            source: std::io::Error::last_os_error(),
        });
    }
    let mut pids = vec![0 as libc::c_int; count as usize + 64];
    let bytes =
        libc::c_int::try_from(pids.len() * size_of::<libc::c_int>()).unwrap_or(libc::c_int::MAX);
    let listed = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    if listed <= 0 {
        return Err(PlatformError::Io {
            context: "listing processes".into(),
            source: std::io::Error::last_os_error(),
        });
    }
    pids.truncate((listed as usize).min(pids.len()));
    Ok(pids
        .into_iter()
        .filter_map(|pid| u32::try_from(pid).ok())
        .filter(|&pid| pid != 0)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_running_test_finds_itself() {
        let own = std::process::id();
        let path = ProcessFinder::get_process_path(own)
            .expect("query")
            .expect("path");
        let name = path.file_name().and_then(OsStr::to_str).expect("name");
        assert!(
            ProcessFinder::find_any_process(&[name])
                .expect("list")
                .is_some()
        );
        assert!(ProcessFinder::process_age(own).is_some());
        assert_eq!(
            ProcessFinder::find_first_thread_id(own).expect("alive"),
            Some(own)
        );
    }
}
