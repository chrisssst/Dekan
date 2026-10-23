use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::error::PlatformError;

pub struct SingleInstanceGuard {
    _lock: File,
}

impl SingleInstanceGuard {
    pub fn acquire(name: &str, state_dir: Option<&Path>) -> Result<Self, PlatformError> {
        let dir = state_dir.map_or_else(std::env::temp_dir, Path::to_path_buf);
        std::fs::create_dir_all(&dir).map_err(|source| PlatformError::Io {
            context: format!("creating the instance folder {}", dir.display()),
            source,
        })?;
        let pid_path = dir.join(format!("{name}.pid"));
        let mut lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&pid_path)
            .map_err(|source| PlatformError::Io {
                context: format!("opening the instance lockfile {}", pid_path.display()),
                source,
            })?;

        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let pid = running_pid(&pid_path);
                info!(
                    ?pid,
                    "Another Dekan instance holds the single-instance lock"
                );
                return Err(PlatformError::AlreadyRunning { pid });
            }
            Err(TryLockError::Error(source)) => {
                return Err(PlatformError::Io {
                    context: format!("locking {}", pid_path.display()),
                    source,
                });
            }
        }

        let current_pid = std::process::id();
        if let Err(e) = lock
            .set_len(0)
            .and_then(|()| lock.write_all(current_pid.to_string().as_bytes()))
        {
            warn!(path = %pid_path.display(), error = %e, "Could not record the instance PID file");
        }

        debug!(name, pid = current_pid, "Single-instance lock acquired");
        Ok(Self { _lock: lock })
    }
}

fn running_pid(pid_path: &PathBuf) -> Option<u32> {
    match std::fs::read_to_string(pid_path) {
        Ok(content) => content.trim().parse().ok(),
        Err(e) => {
            debug!(path = %pid_path.display(), error = %e, "Instance lockfile unreadable while the lock is held");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_guard_on_the_same_name_is_refused_until_the_first_drops() {
        let dir = std::env::temp_dir().join(format!("dekan_instance_{}", std::process::id()));
        let first = SingleInstanceGuard::acquire("probe", Some(&dir)).expect("first");
        assert!(matches!(
            SingleInstanceGuard::acquire("probe", Some(&dir)),
            Err(PlatformError::AlreadyRunning { pid: Some(pid) }) if pid == std::process::id()
        ));
        drop(first);
        assert!(SingleInstanceGuard::acquire("probe", Some(&dir)).is_ok());
        std::fs::remove_dir_all(dir).ok();
    }
}
