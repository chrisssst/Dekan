use std::path::{Path, PathBuf};

use dekan_platform::process::ProcessFinder;
use tracing::{debug, error, info, warn};
use windows::Win32::Foundation::HANDLE;

use crate::error::InjectError;

pub struct SuspendGuard {
    thread_id: u32,
    handle: Option<HANDLE>,
    lock_file: Option<PathBuf>,
}

unsafe impl Send for SuspendGuard {}

impl SuspendGuard {
    pub fn acquire(pid: u32, tid: u32, state_dir: Option<&Path>) -> Result<Self, InjectError> {
        info!(pid = pid, tid = tid, "Suspending game thread");
        let (actual_tid, handle) = ProcessFinder::suspend_process_thread(pid, tid)
            .map_err(|e| InjectError::Suspend(e.to_string()))?;

        if actual_tid != tid {
            debug!(
                pid = pid,
                preferred_tid = tid,
                actual_tid = actual_tid,
                "Suspended alternate game thread after preferred TID was inaccessible"
            );
        }

        let lock_file = state_dir.and_then(|dir| {
            let write = std::fs::create_dir_all(dir)
                .and_then(|()| std::fs::write(dir.join("suspend.lock"), format!("{pid}:{actual_tid}")));
            match write {
                Ok(()) => Some(dir.join("suspend.lock")),
                Err(e) => {
                    warn!(
                        pid = pid,
                        tid = actual_tid,
                        error = %e,
                        "Could not write the suspension sentinel; a crash before resume would need Task Manager"
                    );
                    None
                }
            }
        });

        Ok(Self {
            thread_id: actual_tid,
            handle: Some(handle),
            lock_file,
        })
    }

    pub fn resume(mut self) -> Result<(), InjectError> {
        self.perform_resume()
    }

    fn perform_resume(&mut self) -> Result<(), InjectError> {
        if let Some(handle) = self.handle.take() {
            info!(tid = self.thread_id, "Resuming game thread");
            if let Err(e) = ProcessFinder::resume_thread_raw(handle) {
                error!(tid = self.thread_id, error = %e, "EMERGENCY: Failed to resume game thread!");
                return Err(InjectError::Suspend(format!("resume failed: {e}")));
            }
        }

        if let Some(ref lock_path) = self.lock_file {
            if lock_path.exists() {
                let _ = std::fs::remove_file(lock_path); // ignore-ok: a leftover sentinel only triggers one orphan check on the next boot
            }
        }

        Ok(())
    }
}

impl Drop for SuspendGuard {
    fn drop(&mut self) {
        if self.handle.is_some() {
            warn!(
                tid = self.thread_id,
                "SuspendGuard dropped without explicit resume; auto-resuming now"
            );
            if let Err(e) = self.perform_resume() {
                error!(
                    tid = self.thread_id,
                    error = %e,
                    "EMERGENCY: auto-resume on drop failed; the game may still be suspended"
                );
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanRecovery {
    Clean,

    Corrupt { content: String },

    Gone { pid: u32 },

    NotTheGame { pid: u32, exe: String },

    Resumed { pid: u32, tid: u32 },

    Failed { pid: u32, tid: u32, error: String },
}

pub fn recover_orphaned_suspension(state_dir: &Path, expected_exe: &str) -> OrphanRecovery {
    let lock_file = state_dir.join("suspend.lock");
    let content = match std::fs::read_to_string(&lock_file) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return OrphanRecovery::Clean,
        Err(e) => {
            remove_sentinel(&lock_file);
            return OrphanRecovery::Corrupt {
                content: format!("unreadable: {e}"),
            };
        }
    };
    let Some((pid, tid)) = parse_sentinel(&content) else {
        remove_sentinel(&lock_file);
        return OrphanRecovery::Corrupt { content };
    };

    let exe = match ProcessFinder::get_process_path(pid) {
        Ok(Some(path)) if ProcessFinder::is_running(pid) => path,
        Ok(_) | Err(_) => {
            remove_sentinel(&lock_file);
            return OrphanRecovery::Gone { pid };
        }
    };
    let name = exe
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !name.eq_ignore_ascii_case(expected_exe) {
        remove_sentinel(&lock_file);
        return OrphanRecovery::NotTheGame { pid, exe: name };
    }

    let resumed = ProcessFinder::resume_process_by_pid(pid).or_else(|process_error| {
        if tid == 0 {
            return Err(process_error);
        }
        ProcessFinder::resume_thread_by_id(tid)
            .map(|_| ())
            .map_err(|thread_error| dekan_platform::error::PlatformError::Io {
                context: format!("process: {process_error}; thread {tid}: {thread_error}"),
                source: std::io::Error::other("orphan resume failed"),
            })
    });
    match resumed {
        Ok(()) => {
            remove_sentinel(&lock_file);
            OrphanRecovery::Resumed { pid, tid }
        }
        Err(e) => OrphanRecovery::Failed {
            pid,
            tid,
            error: e.to_string(),
        },
    }
}

fn parse_sentinel(content: &str) -> Option<(u32, u32)> {
    let (pid, tid) = content.trim().split_once(':')?;
    let pid = pid.trim().parse::<u32>().ok().filter(|pid| *pid != 0)?;
    let tid = tid.trim().parse::<u32>().ok()?;
    Some((pid, tid))
}

fn remove_sentinel(lock_file: &Path) {
    if let Err(e) = std::fs::remove_file(lock_file) {
        if e.kind() != std::io::ErrorKind::NotFound {
            warn!(file = %lock_file.display(), error = %e, "Suspension sentinel could not be removed; the next boot checks it again");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "dekan_suspend_{name}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0); // ignore-ok: fixture cleanup
        }
    }

    const PING: &str = r"C:\Windows\System32\PING.EXE";

    fn exits_within(child: &mut std::process::Child, limit: std::time::Duration) -> bool {
        let started = std::time::Instant::now();
        while started.elapsed() < limit {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn test_sentinel_parsing() {
        assert_eq!(parse_sentinel("12345:67890"), Some((12345, 67890)));
        assert_eq!(parse_sentinel(" 1:0\r\n"), Some((1, 0)));
        for bad in ["", "12345", "a:b", "0:5", "1:2:3", "-1:2", "99999999999:1"] {
            assert_eq!(parse_sentinel(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn test_no_sentinel_is_clean_and_a_corrupt_one_is_removed() {
        let dir = TempDir::new("corrupt");
        assert_eq!(
            recover_orphaned_suspension(&dir.0, "League of Legends.exe"),
            OrphanRecovery::Clean
        );
        let lock = dir.0.join("suspend.lock");
        std::fs::write(&lock, "garbage").expect("write");
        assert!(matches!(
            recover_orphaned_suspension(&dir.0, "League of Legends.exe"),
            OrphanRecovery::Corrupt { .. }
        ));
        assert!(
            !lock.exists(),
            "a corrupt sentinel must not come back every boot"
        );
    }

    #[test]
    fn test_a_pid_that_no_longer_exists_is_gone() {
        let dir = TempDir::new("gone");
        let mut child = std::process::Command::new(PING)
            .args(["-n", "1", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn");
        let pid = child.id();
        child.wait().expect("wait");
        std::fs::write(dir.0.join("suspend.lock"), format!("{pid}:0")).expect("write");
        assert_eq!(
            recover_orphaned_suspension(&dir.0, "PING.EXE"),
            OrphanRecovery::Gone { pid }
        );
        assert!(!dir.0.join("suspend.lock").exists());
    }

    #[test]
    fn test_a_pid_now_owned_by_another_program_is_left_alone() {
        let dir = TempDir::new("reuse");
        let mut child = std::process::Command::new(PING)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn");
        let pid = child.id();
        std::fs::write(dir.0.join("suspend.lock"), format!("{pid}:0")).expect("write");
        let outcome = recover_orphaned_suspension(&dir.0, "League of Legends.exe");
        let _ = child.kill(); // ignore-ok: test teardown
        let _ = child.wait(); // ignore-ok: test teardown
        assert!(
            matches!(outcome, OrphanRecovery::NotTheGame { pid: p, ref exe } if p == pid && exe.eq_ignore_ascii_case("PING.EXE")),
            "{outcome:?}"
        );
    }

    #[test]
    fn test_a_process_left_suspended_by_a_dead_dekan_runs_again_after_recovery() {
        let dir = TempDir::new("orphan");

        let mut child = std::process::Command::new(PING)
            .args(["-n", "2", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn");
        let pid = child.id();
        let tid = ProcessFinder::find_first_thread_id(pid)
            .expect("threads")
            .expect("a thread");
        let guard = SuspendGuard::acquire(pid, tid, Some(&dir.0)).expect("suspend");

        std::mem::forget(guard);
        assert!(dir.0.join("suspend.lock").exists());
        assert!(
            !exits_within(&mut child, std::time::Duration::from_secs(3)),
            "the child must really be suspended for this test to mean anything"
        );

        let outcome = recover_orphaned_suspension(&dir.0, "PING.EXE");
        assert_eq!(outcome, OrphanRecovery::Resumed { pid, tid });
        assert!(!dir.0.join("suspend.lock").exists());
        let finished = exits_within(&mut child, std::time::Duration::from_secs(20));
        if !finished {
            let _ = child.kill(); // ignore-ok: test teardown
        }
        assert!(finished, "the recovered process must run to completion");
    }
}
