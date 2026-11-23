use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use dekan_platform::process::ProcessFinder;

use std::sync::Mutex;

use tracing::{debug, warn};

use crate::error::LcuError;

static LAST_PROBLEM: Mutex<Option<String>> = Mutex::new(None);

fn first_report(key: &str) -> bool {
    match LAST_PROBLEM.lock() {
        Ok(last) if last.as_deref() == Some(key) => false,
        Ok(mut last) => {
            *last = Some(key.to_owned());
            true
        }
        Err(_) => true,
    }
}

fn clear_report() {
    if let Ok(mut last) = LAST_PROBLEM.lock() {
        *last = None;
    }
}

macro_rules! warn_once {
    ($key:expr, $($arg:tt)+) => {
        if first_report($key) {
            warn!($($arg)+)
        } else {
            debug!($($arg)+)
        }
    };
}

#[derive(Clone, PartialEq, Eq)]
pub struct Lockfile {
    pub process_name: String,
    pub pid: u32,
    pub port: u16,
    pub auth_token: String,
    pub protocol: String,
}

impl std::fmt::Debug for Lockfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lockfile")
            .field("process_name", &self.process_name)
            .field("pid", &self.pid)
            .field("port", &self.port)
            .field("auth_token", &"<redacted>")
            .field("protocol", &self.protocol)
            .finish()
    }
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
                warn_once!(
                    &format!("read:{}:{:?}", path.display(), e.kind()),
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
                warn_once!(
                    &format!("malformed:{}:{e}", path.display()),
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

            warn_once!(
                &format!("configured:{}", p.display()),
                path = %p.display(),
                "Configured lockfile path does not exist; falling back to discovery"
            );
        }

        if let Ok(env_val) = std::env::var("LCU_LOCKFILE") {
            let env_path = PathBuf::from(env_val);
            if env_path.is_file() {
                debug!(path = %env_path.display(), source = "LCU_LOCKFILE", "Lockfile located");
                return Some(env_path);
            }
            warn_once!(
                &format!("env:{}", env_path.display()),
                path = %env_path.display(),
                "LCU_LOCKFILE is set but points nowhere"
            );
        }

        for proc_name in &dekan_platform::game_version::CLIENT_EXES {
            match ProcessFinder::find_process_path(proc_name) {
                Ok(Some(exe_path)) => {
                    let mut candidates = Vec::with_capacity(8);
                    for ancestor in exe_path.ancestors().skip(1).take(7) {
                        candidates.push(ancestor.join("lockfile"));
                        candidates.push(ancestor.join("Contents/LoL/lockfile"));
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

                    warn_once!(
                        &format!("beside:{}", exe_path.display()),
                        process = proc_name,
                        exe = %exe_path.display(),
                        tried = candidates.len(),
                        "Client process is running but no lockfile was found beside it"
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    warn_once!(
                        &format!("inspect:{proc_name}:{e}"),
                        process = proc_name,
                        error = %e,
                        "Could not inspect processes while looking for the client"
                    );
                }
            }
        }

        for install in dekan_platform::paths::riot_league_installs() {
            for sub in ["lockfile", "Contents/LoL/lockfile"] {
                let candidate = install.join(sub);
                if candidate.is_file() {
                    debug!(path = %candidate.display(), source = "riot-client-installs", "Lockfile located");
                    return Some(candidate);
                }
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
                warn_once!(
                    &format!("alive:{}:{e}", self.pid),
                    pid = self.pid,
                    error = %e,
                    "Could not verify whether the client process is alive; assuming it is"
                );
                true
            }
        }
    }

    #[must_use]
    pub fn parse_ux_cmdline(cmdline: &str, pid: u32) -> Option<Self> {
        let port = extract_flag(cmdline, "--app-port=")?
            .parse::<u16>()
            .ok()
            .filter(|&p| p > 0)?;
        let auth_token = extract_flag(cmdline, "--remoting-auth-token=")?;
        if auth_token.is_empty() {
            return None;
        }
        Some(Self {
            process_name: "LeagueClientUx".to_owned(),
            pid,
            port,
            auth_token,
            protocol: "https".to_owned(),
        })
    }

    pub fn discover(explicit_path: Option<&Path>) -> Result<Self, LcuError> {
        if let Some(path) = Self::find_lockfile_path(explicit_path) {
            if let Ok(lockfile) = Self::from_file(&path) {
                if lockfile.is_client_alive() {
                    clear_report();
                    return Ok(lockfile);
                }
                warn_once!(
                    &format!("stale:{}:{}", path.display(), lockfile.pid),
                    path = %path.display(),
                    pid = lockfile.pid,
                    port = lockfile.port,
                    "Stale lockfile: the client process is gone. Not connecting"
                );
            }
        }
        for proc_name in &dekan_platform::game_version::CLIENT_EXES {
            if let Some(pid) = ProcessFinder::find_process_by_name(proc_name)
                .ok()
                .flatten()
                && let Some(cmd) = dekan_platform::process::query_process_cmdline(pid)
                && let Some(found) = Self::parse_ux_cmdline(&cmd, pid)
            {
                debug!(
                    pid = found.pid,
                    port = found.port,
                    token_len = found.auth_token.len(),
                    "LCU credentials recovered from LeagueClientUx command line"
                );
                clear_report();
                return Ok(found);
            }
        }
        Err(LcuError::LockfileNotFound)
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

fn extract_flag(cmdline: &str, prefix: &str) -> Option<String> {
    let idx = cmdline.find(prefix)?;
    let rest = &cmdline[idx + prefix.len()..];
    let val = if let Some(quoted) = rest.strip_prefix('"') {
        quoted.split('"').next()?
    } else {
        rest.split_whitespace().next()?.trim_matches('"')
    };
    Some(val.to_owned())
}

#[cfg(test)]
mod tests;
