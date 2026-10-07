use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, GetProcessTimes, OpenProcess, PROCESS_NAME_FORMAT,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, SetThreadPriority,
    THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END,
};
use windows::core::PWSTR;

use tracing::debug;

use crate::error::PlatformError;

const FILETIME_UNIX_OFFSET_SECS: u64 = 11_644_473_600;

#[must_use = "the thread leaves background mode when this is dropped"]
pub struct BackgroundThread(());

impl BackgroundThread {
    pub fn enter() -> Option<Self> {
        match unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) } {
            Ok(()) => Some(Self(())),
            Err(e) => {
                debug!(error = %e, "Background thread mode unavailable; the work runs at normal priority");
                None
            }
        }
    }
}

impl Drop for BackgroundThread {
    fn drop(&mut self) {
        let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END) }; // ignore-ok: the thread is ending its background work; a failure only leaves it at low priority
    }
}
const FILETIME_TICKS_PER_SEC: u64 = 10_000_000;

#[must_use]
pub fn filetime_age(created_ticks: u64, now: std::time::SystemTime) -> Option<std::time::Duration> {
    let since_unix = now.duration_since(std::time::UNIX_EPOCH).ok()?;
    let now_ticks = since_unix
        .as_secs()
        .checked_add(FILETIME_UNIX_OFFSET_SECS)?
        .checked_mul(FILETIME_TICKS_PER_SEC)?
        .checked_add(u64::from(since_unix.subsec_nanos()) / 100)?;
    let ticks = now_ticks.checked_sub(created_ticks)?;
    Some(std::time::Duration::from_nanos(ticks.saturating_mul(100)))
}

pub struct ProcessFinder;

impl ProcessFinder {
    #[must_use]
    pub fn process_age(pid: u32) -> Option<std::time::Duration> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let times =
            unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) };
        unsafe {
            let _ = CloseHandle(handle); // ignore-ok: handle released after the query; nothing to recover from
        };
        times.ok()?;
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        filetime_age(ticks, std::time::SystemTime::now())
    }

    pub fn get_process_path(pid: u32) -> Result<Option<PathBuf>, PlatformError> {
        let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
            Ok(h) => h,
            Err(_) => return Ok(None),
        };

        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;

        let res = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_FORMAT(0),
                PWSTR(buffer.as_mut_ptr()),
                &mut size,
            )
        };

        unsafe {
            let _ = CloseHandle(handle); // ignore-ok: handle released at teardown; nothing to recover from
        };

        if res.is_ok() && size > 0 {
            let path_os = OsString::from_wide(&buffer[..size as usize]);
            Ok(Some(PathBuf::from(path_os)))
        } else {
            Ok(None)
        }
    }

    pub fn find_process_path(exe_name: &str) -> Result<Option<PathBuf>, PlatformError> {
        if let Some(pid) = Self::find_process_by_name(exe_name)? {
            Self::get_process_path(pid)
        } else {
            Ok(None)
        }
    }
    pub fn find_process_by_name(exe_name: &str) -> Result<Option<u32>, PlatformError> {
        Self::find_any_process(&[exe_name])
    }

    pub fn find_any_process(exe_names: &[&str]) -> Result<Option<u32>, PlatformError> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?;

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut has_next = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();

        let mut found_pid = None;
        while has_next {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name_os = OsString::from_wide(&entry.szExeFile[..len]);

            if let Some(name_str) = name_os.to_str() {
                if exe_names
                    .iter()
                    .any(|exe| name_str.eq_ignore_ascii_case(exe))
                {
                    found_pid = Some(entry.th32ProcessID);
                    break;
                }
            }

            has_next = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }

        unsafe {
            let _ = CloseHandle(snapshot); // ignore-ok: the snapshot is released either way
        };

        Ok(found_pid)
    }

    pub fn find_first_thread_id(pid: u32) -> Result<Option<u32>, PlatformError> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }?;

        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };

        let mut has_next = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();

        let mut found_tid = None;
        while has_next {
            if entry.th32OwnerProcessID == pid {
                found_tid = Some(entry.th32ThreadID);
                break;
            }

            has_next = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
        }

        unsafe {
            let _ = CloseHandle(snapshot); // ignore-ok: the snapshot is released either way
        };

        if found_tid.is_none() {
            debug!(pid, "No thread found for this PID; the process is gone");
        }

        Ok(found_tid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_mode_is_entered_once_and_left_on_drop() {
        let guard = BackgroundThread::enter().expect("background mode");
        assert!(
            BackgroundThread::enter().is_none(),
            "Windows refuses a nested background mode"
        );
        drop(guard);
        let again = BackgroundThread::enter();
        assert!(again.is_some(), "leaving restores normal mode");
    }

    #[test]
    fn test_find_current_process() {
        let current_pid = std::process::id();
        let found = ProcessFinder::find_first_thread_id(current_pid).unwrap();
        assert!(found.is_some(), "should find thread for current process");
    }

    #[test]
    fn test_non_existent_process() {
        let found =
            ProcessFinder::find_process_by_name("non_existent_dekan_process_xyz123.exe").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_get_current_process_path() {
        let current_pid = std::process::id();
        let path = ProcessFinder::get_process_path(current_pid).unwrap();
        assert!(path.is_some(), "should resolve current process binary path");
        let path = path.unwrap();
        assert!(path.exists(), "process path must exist on disk");
    }

    #[test]
    fn test_the_current_process_has_a_short_age() {
        let age = ProcessFinder::process_age(std::process::id()).expect("own process age");
        assert!(age < std::time::Duration::from_secs(600));
    }

    #[test]
    fn test_filetime_age_counts_from_1601() {
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(10);
        let created =
            FILETIME_UNIX_OFFSET_SECS * FILETIME_TICKS_PER_SEC + 4 * FILETIME_TICKS_PER_SEC;
        assert_eq!(
            filetime_age(created, now),
            Some(std::time::Duration::from_secs(6))
        );
        assert_eq!(
            filetime_age(u64::MAX, now),
            None,
            "a creation time in the future has no age"
        );
    }

    #[test]
    fn test_a_missing_process_has_no_age() {
        assert_eq!(ProcessFinder::process_age(u32::MAX), None);
    }
}
