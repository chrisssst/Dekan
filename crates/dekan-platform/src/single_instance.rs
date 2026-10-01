use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::HSTRING;

use tracing::{debug, info, warn};

use crate::error::PlatformError;
use crate::process::ProcessFinder;

pub struct SingleInstanceGuard {
    mutex_handle: HANDLE,
    pid_file: Option<PathBuf>,
}

impl SingleInstanceGuard {
    pub fn acquire(name: &str, state_dir: Option<&Path>) -> Result<Self, PlatformError> {
        let mutex_name = format!("Local\\Dekan_SingleInstance_{name}");
        let hstring_name = HSTRING::from(mutex_name);

        let handle = unsafe { CreateMutexW(None, true, &hstring_name) }?;

        let last_error = unsafe { GetLastError() };

        if last_error == ERROR_ALREADY_EXISTS {
            if let Some(dir) = state_dir {
                let pid_path = dir.join(format!("{name}.pid"));
                if pid_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&pid_path) {
                        if let Ok(existing_pid) = content.trim().parse::<u32>() {
                            if let Ok(Some(_)) = ProcessFinder::find_first_thread_id(existing_pid) {
                                info!(
                                    pid = existing_pid,
                                    "Another Dekan instance holds the single-instance lock"
                                );

                                unsafe {
                                    let _ = CloseHandle(handle); // ignore-ok: handle released at teardown; nothing to recover from
                                };
                                return Err(PlatformError::AlreadyRunning {
                                    pid: Some(existing_pid),
                                });
                            }

                            warn!(
                                pid = existing_pid,
                                path = %pid_path.display(),
                                "Removing a stale instance lockfile left by a process that died"
                            );
                            let _ = std::fs::remove_file(&pid_path); // ignore-ok: a stale file only costs this log line next boot
                        }
                    }
                }
            }

            unsafe {
                let _ = CloseHandle(handle); // ignore-ok: handle released at teardown; nothing to recover from
            };
            return Err(PlatformError::AlreadyRunning { pid: None });
        }

        let pid_file = if let Some(dir) = state_dir {
            if !dir.exists() {
                let _ = std::fs::create_dir_all(dir); // ignore-ok: the PID file write below reports the real outcome
            }
            let file = dir.join(format!("{name}.pid"));
            let current_pid = std::process::id();
            if let Err(e) = std::fs::write(&file, current_pid.to_string()) {
                warn!(path = %file.display(), error = %e, "Could not record the instance PID file");
            }
            Some(file)
        } else {
            None
        };

        debug!(
            name = name,
            pid = std::process::id(),
            "Single-instance lock acquired"
        );

        Ok(Self {
            mutex_handle: handle,
            pid_file,
        })
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if let Some(ref path) = self.pid_file {
            if path.exists() {
                let _ = std::fs::remove_file(path); // ignore-ok: dropping our own PID file at teardown
            }
        }

        unsafe {
            let _ = CloseHandle(self.mutex_handle); // ignore-ok: handle released at teardown; nothing to recover from
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_instance_exclusivity() {
        let temp_dir = std::env::temp_dir().join("dekan_test_single_instance");
        let unique_name = format!("test_{}", std::process::id());

        let guard1 = SingleInstanceGuard::acquire(&unique_name, Some(&temp_dir))
            .expect("first instance should acquire");

        let guard2 = SingleInstanceGuard::acquire(&unique_name, Some(&temp_dir));
        assert!(matches!(guard2, Err(PlatformError::AlreadyRunning { .. })));

        drop(guard1);

        let guard3 = SingleInstanceGuard::acquire(&unique_name, Some(&temp_dir));
        assert!(
            guard3.is_ok(),
            "should acquire after previous guard was dropped"
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }
}
