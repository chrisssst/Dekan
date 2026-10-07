use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use dekan_platform::process::ProcessFinder;

use tracing::{debug, warn};

use crate::error::LcuError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lockfile {
    pub process_name: String,

    pub pid: u32,

    pub port: u16,

    pub auth_token: String,

    pub protocol: String,
}

impl Lockfile {
    pub fn parse(content: &str) -> Result<Self, LcuError> {
        let trimmed = content.trim();
        let parts: Vec<&str> = trimmed.split(':').collect();

        if parts.len() != 5 {
            return Err(LcuError::InvalidLockfile(format!(
                "expected 5 colon-separated fields, got {}",
                parts.len()
            )));
        }

        let process_name = parts[0].to_string();
        let pid = parts[1]
            .parse::<u32>()
            .map_err(|e| LcuError::InvalidLockfile(format!("invalid PID '{}': {e}", parts[1])))?;
        let port = parts[2]
            .parse::<u16>()
            .map_err(|e| LcuError::InvalidLockfile(format!("invalid port '{}': {e}", parts[2])))?;
        let auth_token = parts[3].to_string();
        let protocol = parts[4].to_string();

        if auth_token.is_empty() {
            return Err(LcuError::InvalidLockfile("auth token is empty".into()));
        }
        if port == 0 {
            return Err(LcuError::InvalidLockfile("port 0".into()));
        }

        Ok(Self {
            process_name,
            pid,
            port,
            auth_token,
            protocol,
        })
    }

    pub fn from_file(path: &Path) -> Result<Self, LcuError> {
        if !path.exists() {
            debug!(path = %path.display(), "Lockfile path does not exist");
            return Err(LcuError::LockfileNotFound);
        }

        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,

            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    kind = ?e.kind(),
                    "Lockfile exists but could not be read"
                );
                return Err(e.into());
            }
        };

        match Self::parse(&content) {
            Ok(lockfile) => {
                debug!(
                    path = %path.display(),
                    process = %lockfile.process_name,
                    pid = lockfile.pid,
                    port = lockfile.port,
                    protocol = %lockfile.protocol,

                    token_len = lockfile.auth_token.len(),
                    "Lockfile parsed"
                );
                Ok(lockfile)
            }
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    bytes = content.len(),
                    "Lockfile is present but malformed"
                );
                Err(e)
            }
        }
    }

    pub fn find_lockfile_path(explicit_path: Option<&Path>) -> Option<PathBuf> {
        if let Some(p) = explicit_path {
            if p.is_file() {
                debug!(path = %p.display(), source = "explicit", "Lockfile located");
                return Some(p.to_path_buf());
            }

            warn!(path = %p.display(), "Configured lockfile path does not exist; falling back to discovery");
        }

        if let Ok(env_val) = std::env::var("LCU_LOCKFILE") {
            let env_path = PathBuf::from(env_val);
            if env_path.is_file() {
                debug!(path = %env_path.display(), source = "LCU_LOCKFILE", "Lockfile located");
                return Some(env_path);
            }
            warn!(path = %env_path.display(), "LCU_LOCKFILE is set but points nowhere");
        }

        for proc_name in &dekan_platform::game_version::CLIENT_EXES {
            match ProcessFinder::find_process_path(proc_name) {
                Ok(Some(exe_path)) => {
                    let mut candidates = Vec::with_capacity(2);
                    if let Some(parent) = exe_path.parent() {
                        candidates.push(parent.join("lockfile"));
                        if let Some(grandparent) = parent.parent() {
                            candidates.push(grandparent.join("lockfile"));
                        }
                    }

                    for candidate in &candidates {
                        if candidate.is_file() {
                            debug!(
                                path = %candidate.display(),
                                source = proc_name,
                                "Lockfile located next to the running client"
                            );
                            return Some(candidate.clone());
                        }
                    }

                    warn!(
                        process = proc_name,
                        exe = %exe_path.display(),
                        tried = candidates.len(),
                        "Client process is running but no lockfile was found beside it"
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(process = proc_name, error = %e, "Could not inspect processes while looking for the client");
                }
            }
        }

        let default_roots = [
            r"C:\Riot Games\League of Legends",
            r"D:\Riot Games\League of Legends",
            r"E:\Riot Games\League of Legends",
            r"C:\Program Files\Riot Games\League of Legends",
            r"C:\Program Files (x86)\Riot Games\League of Legends",
        ];

        for root in &default_roots {
            let candidate = Path::new(root).join("lockfile");
            if candidate.is_file() {
                debug!(path = %candidate.display(), source = "default-root", "Lockfile located");
                return Some(candidate);
            }
        }

        debug!("No lockfile found by any resolution step");
        None
    }

    #[must_use]
    pub fn is_client_alive(&self) -> bool {
        match ProcessFinder::find_first_thread_id(self.pid) {
            Ok(Some(_)) => true,
            Ok(None) => false,

            Err(e) => {
                warn!(
                    pid = self.pid,
                    error = %e,
                    "Could not verify whether the client process is alive; assuming it is"
                );
                true
            }
        }
    }

    pub fn discover(explicit_path: Option<&Path>) -> Result<Self, LcuError> {
        let path = Self::find_lockfile_path(explicit_path).ok_or(LcuError::LockfileNotFound)?;
        let lockfile = Self::from_file(&path)?;

        if !lockfile.is_client_alive() {
            warn!(
                path = %path.display(),
                pid = lockfile.pid,
                port = lockfile.port,
                "Stale lockfile: the client process is gone. Not connecting"
            );
            return Err(LcuError::LockfileNotFound);
        }

        Ok(lockfile)
    }

    #[must_use]
    pub fn basic_auth_header(&self) -> String {
        let credentials = format!("riot:{}", self.auth_token);
        let encoded = BASE64.encode(credentials.as_bytes());
        format!("Basic {encoded}")
    }

    #[must_use]
    pub fn base_url(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }

    #[must_use]
    pub fn ws_url(&self) -> String {
        format!("wss://127.0.0.1:{}", self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_lockfile() {
        let content = "LeagueClient:12345:55667:supersecrettoken:https\n";
        let lockfile = Lockfile::parse(content).expect("valid lockfile");

        assert_eq!(lockfile.process_name, "LeagueClient");
        assert_eq!(lockfile.pid, 12345);
        assert_eq!(lockfile.port, 55667);
        assert_eq!(lockfile.auth_token, "supersecrettoken");
        assert_eq!(lockfile.protocol, "https");

        assert_eq!(lockfile.base_url(), "https://127.0.0.1:55667");
        assert_eq!(lockfile.ws_url(), "wss://127.0.0.1:55667");

        let auth = lockfile.basic_auth_header();
        assert!(auth.starts_with("Basic "));
        let raw_b64 = &auth["Basic ".len()..];
        let decoded = String::from_utf8(BASE64.decode(raw_b64).unwrap()).unwrap();
        assert_eq!(decoded, "riot:supersecrettoken");
    }

    #[test]
    fn test_rejects_missing_fields() {
        let content = "LeagueClient:12345:55667:token";
        let err = Lockfile::parse(content).unwrap_err();
        match err {
            LcuError::InvalidLockfile(msg) => assert!(msg.contains("expected 5")),
            other => panic!("expected InvalidLockfile, got {other:?}"),
        }
    }

    #[test]
    fn test_rejects_invalid_port() {
        let content = "LeagueClient:12345:notaport:token:https";
        let err = Lockfile::parse(content).unwrap_err();
        match err {
            LcuError::InvalidLockfile(msg) => assert!(msg.contains("invalid port")),
            other => panic!("expected InvalidLockfile, got {other:?}"),
        }
    }

    #[test]
    fn test_rejects_empty_token() {
        let content = "LeagueClient:12345:55667::https";
        let err = Lockfile::parse(content).unwrap_err();
        match err {
            LcuError::InvalidLockfile(msg) => assert!(msg.contains("token is empty")),
            other => panic!("expected InvalidLockfile, got {other:?}"),
        }
    }

    #[test]
    fn test_find_lockfile_path_explicit() {
        let dir = std::env::temp_dir().join(format!("dekan_test_lock_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir); // ignore-ok: test temp dir
        let lock_path = dir.join("lockfile");

        let live_pid = std::process::id();
        std::fs::write(
            &lock_path,
            format!("LeagueClient:{live_pid}:456:secret:https"),
        )
        .unwrap();

        let found = Lockfile::find_lockfile_path(Some(&lock_path));
        assert_eq!(found, Some(lock_path.clone()));

        let lock = Lockfile::discover(Some(&lock_path)).unwrap();
        assert_eq!(lock.pid, live_pid);
        assert_eq!(lock.port, 456);
        assert_eq!(lock.auth_token, "secret");

        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: test temp dir teardown
    }
}
