use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::Foundation::{CloseHandle, HANDLE, NTSTATUS};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, OpenThread, PROCESS_NAME_FORMAT,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SUSPEND_RESUME, QueryFullProcessImageNameW,
    ResumeThread, SuspendThread, THREAD_SUSPEND_RESUME,
};
use windows::core::{PWSTR, s};

use tracing::{debug, info, warn};

use crate::error::PlatformError;

type NtSuspendProcessFn = unsafe extern "system" fn(process_handle: HANDLE) -> NTSTATUS;
type NtResumeProcessFn = unsafe extern "system" fn(process_handle: HANDLE) -> NTSTATUS;

#[allow(clippy::missing_transmute_annotations)]
fn get_nt_suspend_process() -> Option<NtSuspendProcessFn> {
    unsafe {
        let ntdll =
            windows::Win32::System::LibraryLoader::GetModuleHandleA(s!("ntdll.dll")).ok()?;
        let proc =
            windows::Win32::System::LibraryLoader::GetProcAddress(ntdll, s!("NtSuspendProcess"))?;
        Some(std::mem::transmute(proc))
    }
}

#[allow(clippy::missing_transmute_annotations)]
fn get_nt_resume_process() -> Option<NtResumeProcessFn> {
    unsafe {
        let ntdll =
            windows::Win32::System::LibraryLoader::GetModuleHandleA(s!("ntdll.dll")).ok()?;
        let proc =
            windows::Win32::System::LibraryLoader::GetProcAddress(ntdll, s!("NtResumeProcess"))?;
        Some(std::mem::transmute(proc))
    }
}

fn nt_status_name(status: i32) -> &'static str {
    match status as u32 {
        0xC0000022 => "STATUS_ACCESS_DENIED",
        0xC0000008 => "STATUS_INVALID_HANDLE",
        0xC000000D => "STATUS_INVALID_PARAMETER",
        0xC0000001 => "STATUS_UNSUCCESSFUL",
        0xC0000241 => "STATUS_PROCESS_IS_TERMINATING",
        _ => "unknown NTSTATUS",
    }
}

/// Utilities for discovering and manipulating League game processes and threads.
pub struct ProcessFinder;

impl ProcessFinder {
    /// Retrieve the full executable file path of a process by its PID.
    pub fn get_process_path(pid: u32) -> Result<Option<PathBuf>, PlatformError> {
        // SAFETY: OpenProcess with PROCESS_QUERY_LIMITED_INFORMATION to read the image path.
        let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
            Ok(h) => h,
            Err(_) => return Ok(None),
        };

        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;

        // SAFETY: QueryFullProcessImageNameW writes the null-terminated wide string into buffer.
        let res = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_FORMAT(0),
                PWSTR(buffer.as_mut_ptr()),
                &mut size,
            )
        };

        // SAFETY: Always close the process handle.
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

    /// Find the executable path of a process by its name (e.g. "LeagueClient.exe").
    pub fn find_process_path(exe_name: &str) -> Result<Option<PathBuf>, PlatformError> {
        if let Some(pid) = Self::find_process_by_name(exe_name)? {
            Self::get_process_path(pid)
        } else {
            Ok(None)
        }
    }
    /// Find the Process ID (PID) of an executable by image name (e.g. "League of Legends.exe").
    pub fn find_process_by_name(exe_name: &str) -> Result<Option<u32>, PlatformError> {
        // SAFETY: CreateToolhelp32Snapshot with TH32CS_SNAPPROCESS takes a snapshot of all processes.
        // The returned handle is checked for INVALID_HANDLE_VALUE and closed before return.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?;

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        // SAFETY: Process32FirstW is called with a valid snapshot handle and initialized struct.
        let mut has_next = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();

        let mut found_pid = None;
        while has_next {
            // Find nul-terminator in UTF-16 array
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name_os = OsString::from_wide(&entry.szExeFile[..len]);

            if let Some(name_str) = name_os.to_str() {
                if name_str.eq_ignore_ascii_case(exe_name) {
                    found_pid = Some(entry.th32ProcessID);
                    break;
                }
            }

            // SAFETY: Process32NextW is called on valid snapshot handle until failure.
            has_next = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }

        // SAFETY: Close the snapshot handle.
        unsafe {
            let _ = CloseHandle(snapshot); // ignore-ok: the snapshot is released either way
        };

        match found_pid {
            Some(pid) => debug!(exe = exe_name, pid, "Process found"),
            // Not finding the game or the client is an ordinary state (they are not running), so
            // this stays at debug — but it is no longer invisible.
            None => debug!(exe = exe_name, "Process not running"),
        }

        Ok(found_pid)
    }

    /// Find the first thread ID belonging to a given process ID.
    pub fn find_first_thread_id(pid: u32) -> Result<Option<u32>, PlatformError> {
        // SAFETY: CreateToolhelp32Snapshot with TH32CS_SNAPTHREAD takes a snapshot of all threads.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }?;

        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };

        // SAFETY: Thread32First called on valid snapshot.
        let mut has_next = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();

        let mut found_tid = None;
        while has_next {
            if entry.th32OwnerProcessID == pid {
                found_tid = Some(entry.th32ThreadID);
                break;
            }

            // SAFETY: Thread32Next called on valid snapshot.
            has_next = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
        }

        // SAFETY: Close snapshot handle.
        unsafe {
            let _ = CloseHandle(snapshot); // ignore-ok: the snapshot is released either way
        };

        if found_tid.is_none() {
            // No thread for a PID means the process is gone. Callers use this as a liveness probe
            // (stale lockfile, stale instance lock), so the distinction matters.
            debug!(pid, "No thread found for this PID; the process is gone");
        }

        Ok(found_tid)
    }

    /// Attempt to suspend a process or thread belonging to `pid`.
    ///
    /// Prioritizes process-level suspension via `NtSuspendProcess`.
    /// On modern Windows with Vanguard / anti-cheat, individual threads are often protected from
    /// `OpenThread(THREAD_SUSPEND_RESUME)` callbacks, while elevated process-level suspension succeeds.
    ///
    /// If `NtSuspendProcess` fails, falls back to attempting `preferred_tid` and other threads.
    pub fn suspend_process_thread(
        pid: u32,
        preferred_tid: u32,
    ) -> Result<(u32, HANDLE), PlatformError> {
        // 1. Try process-level suspension via NtSuspendProcess first (only for external processes).
        // Suspending the current process freezes all threads including the calling thread and runtime!
        if pid != std::process::id() {
            // SAFETY: OpenProcess with PROCESS_SUSPEND_RESUME on an external PID (checked above);
            // the handle is closed on every path below, including the early-fallthrough one.
            // Requires `SeDebugPrivilege` on our token to succeed against a Vanguard-protected
            // process — enabled once for the process lifetime in `main.rs` (see `elevation.rs`).
            // Newly spawned processes may take a few milliseconds to complete primary token initialization,
            // so we retry up to 10 times with a 25ms delay (250ms total).
            let mut last_open_err = None;
            let mut last_nt_status: Option<i32> = None;
            let mut attempts = 0u32;
            for attempt in 1..=10 {
                attempts = attempt;
                let open_result = unsafe { OpenProcess(PROCESS_SUSPEND_RESUME, false, pid) };
                match open_result {
                    Ok(process_handle) => {
                        if let Some(nt_suspend) = get_nt_suspend_process() {
                            // SAFETY: process_handle was just opened above with PROCESS_SUSPEND_RESUME.
                            let status = unsafe { nt_suspend(process_handle) };
                            if status.0 >= 0 {
                                info!(
                                    pid,
                                    attempt, "Process suspended atomically via NtSuspendProcess"
                                );
                                return Ok((preferred_tid, process_handle));
                            }
                            // Reported once after the loop, not per attempt: ten identical lines
                            // for one denial is the polling noise `coding-standards.md` forbids.
                            last_nt_status = Some(status.0);
                        }
                        unsafe {
                            let _ = CloseHandle(process_handle); // ignore-ok: handle closed on cleanup
                        };
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    Err(e) => {
                        last_open_err = Some(e);
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                }
            }

            if let Some(status) = last_nt_status {
                warn!(
                    pid,
                    attempts,
                    status = format!("0x{:08X}", status as u32),
                    meaning = nt_status_name(status),
                    "NtSuspendProcess refused by the kernel; falling back to thread-level suspension"
                );
            }

            if let Some(e) = last_open_err {
                warn!(
                    pid,
                    attempts,
                    error = %e,
                    "OpenProcess(PROCESS_SUSPEND_RESUME) failed after retries; falling back to thread-level suspension"
                );
            }
        }

        // 2. Fallback to thread-level suspension:
        if let Ok(handle) = Self::suspend_thread_raw(preferred_tid) {
            return Ok((preferred_tid, handle));
        }

        // SAFETY: Snapshot system threads to search for any other thread of the same PID.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }?;
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };

        let mut has_next = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();
        let mut last_err = None;

        while has_next {
            if entry.th32OwnerProcessID == pid && entry.th32ThreadID != preferred_tid {
                match Self::suspend_thread_raw(entry.th32ThreadID) {
                    Ok(handle) => {
                        unsafe {
                            let _ = CloseHandle(snapshot); // ignore-ok: snapshot handle released
                        };
                        return Ok((entry.th32ThreadID, handle));
                    }
                    Err(e) => {
                        last_err = Some(e);
                    }
                }
            }
            has_next = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
        }

        unsafe {
            let _ = CloseHandle(snapshot); // ignore-ok: snapshot handle released
        };

        Err(last_err.unwrap_or_else(|| PlatformError::ProcessNotFound {
            name: format!("No suspendable thread or process found for PID {pid}"),
        }))
    }

    /// Open and suspend a thread by its thread ID.
    /// Returns the raw thread handle which must be closed and resumed.
    pub fn suspend_thread_raw(tid: u32) -> Result<HANDLE, PlatformError> {
        // SAFETY: OpenThread with THREAD_SUSPEND_RESUME permissions.
        let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, tid) }?;

        // SAFETY: SuspendThread suspends execution of the target thread.
        let prev_count = unsafe { SuspendThread(handle) };
        if prev_count == u32::MAX {
            // SAFETY: Close handle on failure
            unsafe {
                let _ = CloseHandle(handle); // ignore-ok: handle released at teardown; nothing to recover from
            };
            return Err(PlatformError::Io {
                context: format!("failed to suspend thread {tid}"),
                source: std::io::Error::last_os_error(),
            });
        }

        Ok(handle)
    }

    /// Whether `pid` names a process that has not exited. A process that exited stays an object
    /// (and keeps its image path) while any handle to it is open, so existence is not enough.
    #[must_use]
    pub fn is_running(pid: u32) -> bool {
        /// `STILL_ACTIVE`: the exit code of a process that has not exited.
        const STILL_ACTIVE: u32 = 259;
        // SAFETY: OpenProcess with PROCESS_QUERY_LIMITED_INFORMATION; closed below.
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return false;
        };
        let mut code = 0u32;
        // SAFETY: `handle` is valid and `code` outlives the call.
        let queried = unsafe { GetExitCodeProcess(handle, &mut code) };
        unsafe {
            let _ = CloseHandle(handle); // ignore-ok: the query result below is what matters
        };
        queried.is_ok() && code == STILL_ACTIVE
    }

    /// Undo one `NtSuspendProcess` on `pid` from a fresh handle: the recovery path, when the handle
    /// that suspended it died with an earlier Dekan process.
    ///
    /// `NtResumeProcess` lowers every thread's suspend count by one and leaves threads at zero
    pub fn resume_process_by_pid(pid: u32) -> Result<(), PlatformError> {
        let resume = get_nt_resume_process().ok_or_else(|| PlatformError::Io {
            context: "NtResumeProcess is not exported by ntdll".into(),
            source: std::io::Error::other("missing export"),
        })?;

        let handle = unsafe { OpenProcess(PROCESS_SUSPEND_RESUME, false, pid) }?;

        let status = unsafe { resume(handle) };
        unsafe {
            let _ = CloseHandle(handle); // ignore-ok: handle released at teardown; nothing to recover from
        };
        if status.0 < 0 {
            return Err(PlatformError::Io {
                context: format!(
                    "NtResumeProcess({pid}) returned 0x{:08X} ({})",
                    status.0 as u32,
                    nt_status_name(status.0)
                ),
                source: std::io::Error::other("NTSTATUS failure"),
            });
        }
        Ok(())
    }

    pub fn resume_thread_by_id(tid: u32) -> Result<u32, PlatformError> {
        let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, tid) }?;

        let previous = unsafe { ResumeThread(handle) };
        let error = std::io::Error::last_os_error();
        unsafe {
            let _ = CloseHandle(handle); // ignore-ok: handle released at teardown; nothing to recover from
        };
        if previous == u32::MAX {
            return Err(PlatformError::Io {
                context: format!("failed to resume thread {tid}"),
                source: error,
            });
        }
        Ok(previous)
    }

    pub fn resume_thread_raw(handle: HANDLE) -> Result<u32, PlatformError> {
        if let Some(nt_resume) = get_nt_resume_process() {
            let status = unsafe { nt_resume(handle) };
            if status.0 >= 0 {
                unsafe {
                    let _ = CloseHandle(handle); // ignore-ok: the query result below is what matters
                };
                info!("Process resumed cleanly via NtResumeProcess");
                return Ok(0);
            }
        }

        let count = unsafe { ResumeThread(handle) };

        unsafe {
            let _ = CloseHandle(handle); // ignore-ok: the resume status below is what is reported
        };

        if count == u32::MAX {
            return Err(PlatformError::Io {
                context: "failed to resume thread".into(),
                source: std::io::Error::last_os_error(),
            });
        }

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_suspend_and_resume_child_process() {
        let mut child = std::process::Command::new(r"C:\Windows\System32\PING.EXE")
            .args(["-n", "10", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn child process");

        let pid = child.id();
        let (actual_tid, handle) = ProcessFinder::suspend_process_thread(pid, 0)
            .expect("suspend_process_thread must succeed on child process");

        assert_eq!(actual_tid, 0);

        ProcessFinder::resume_thread_raw(handle).expect("resume_thread_raw must succeed");

        let _ = child.kill(); // ignore-ok: best-effort child teardown in test
        let _ = child.wait(); // ignore-ok: wait for child exit in test
    }
}
