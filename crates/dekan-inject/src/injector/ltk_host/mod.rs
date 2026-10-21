pub const HOST_EXE: &str = "ltk_patcher_host.exe";

pub const DLL_FILE: &str = "ltk_patcher_dll.dll";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostLogLevel {
    Error = 0,

    Info = 0x10,

    Debug = 0x20,
}

impl HostLogLevel {
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var(dekan_core::env::LOG) {
            Ok(v) if v.eq_ignore_ascii_case("trace") || v.eq_ignore_ascii_case("debug") => {
                Self::Debug
            }
            _ => Self::Info,
        }
    }
}

pub mod hook_flags {

    pub const DISABLE_VERIFY: u32 = 1;

    pub const DISABLE_FILE: u32 = 2;

    pub const OPT_OUT_AH_V1: u32 = 4;

    pub const FULL_WAD_SCAN: u32 = 8;
}

#[must_use]
pub fn default_flags() -> u32 {
    flags_from(
        std::env::var(dekan_core::env::PATCHER_FLAGS)
            .ok()
            .as_deref(),
    )
}

#[must_use]
pub fn flags_from(value: Option<&str>) -> u32 {
    let Some(value) = value else {
        return hook_flags::OPT_OUT_AH_V1;
    };
    match value.trim().parse() {
        Ok(flags) => flags,
        Err(e) => {
            tracing::warn!(
                value,
                error = %e,
                default = hook_flags::OPT_OUT_AH_V1,
                "DEKAN_PATCHER_FLAGS is not a number; using the default hook flags"
            );
            hook_flags::OPT_OUT_AH_V1
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostState {
    Injecting,

    Injected,

    Waiting,

    Exited,

    Failed,
}

impl HostState {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "injecting" => Some(Self::Injecting),
            "injected" => Some(Self::Injected),
            "waiting" => Some(Self::Waiting),
            "exited" => Some(Self::Exited),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    Ok { message: String },

    Status { state: HostState, message: String },

    Error { message: String },

    DllLog { level: String, message: String },
}

fn first_token(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find([' ', '\t']) {
        Some(pos) => (&s[..pos], s[pos + 1..].trim_start()),
        None => (s, ""),
    }
}

#[must_use]
pub fn parse_host_event(line: &str) -> Option<HostEvent> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return None;
    }

    let (keyword, rest) = first_token(line);
    match keyword {
        "ok" => {
            let (_ts, message) = first_token(rest);
            Some(HostEvent::Ok {
                message: message.to_owned(),
            })
        }
        "status" => {
            let (_ts, after_ts) = first_token(rest);
            let (state_str, message) = first_token(after_ts);
            let state = HostState::parse(state_str)?;
            Some(HostEvent::Status {
                state,
                message: message.to_owned(),
            })
        }
        "error" => {
            let (_ts, message) = first_token(rest);
            Some(HostEvent::Error {
                message: message.to_owned(),
            })
        }
        "dll" => {
            let (_ts, a) = first_token(rest);
            let (_pid, b) = first_token(a);
            let (_tid, c) = first_token(b);
            let (level, message) = first_token(c);
            if level.is_empty() {
                return None;
            }
            Some(HostEvent::DllLog {
                level: level.to_owned(),
                message: message.to_owned(),
            })
        }
        _ => None,
    }
}

#[must_use]
pub fn redirected_wad(message: &str) -> Option<&str> {
    message
        .split_once("redirected wad:")
        .map(|(_, wad)| wad.trim())
        .filter(|wad| !wad.is_empty())
}

#[must_use]
pub fn is_end_of_life(message: &str) -> bool {
    message.contains("end of life reached")
}

#[must_use]
pub fn is_expected_status(message: &str) -> bool {
    message.contains("c0000229")
}

#[must_use]
pub fn joined_too_late(message: &str) -> bool {
    message.contains("joined too late")
}

const TERMINATED_EXTERNALLY: u32 = 0x4001_0004;

#[must_use]
pub fn closed_by_another_program(exit_code: i32) -> bool {
    exit_code as u32 == TERMINATED_EXTERNALLY
}

#[must_use]
pub fn is_dll_failure(level: &str, message: &str) -> bool {
    if is_expected_status(message) {
        return false;
    }
    if level.eq_ignore_ascii_case("error") {
        return true;
    }
    let lower = message.to_ascii_lowercase();
    lower.contains("failed") || lower.contains("disabling overlay") || is_end_of_life(message)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DllSupport {
    Supported,

    SupportedUntilNextPatch { days_left: i64 },

    Refused,
}

#[must_use]
pub fn dll_support(stamp: u32, limit: u32, now: u64) -> DllSupport {
    if stamp > limit {
        return DllSupport::Refused;
    }
    let days_left = (i64::from(limit) - now as i64).div_euclid(86_400);
    if days_left <= 7 {
        DllSupport::SupportedUntilNextPatch { days_left }
    } else {
        DllSupport::Supported
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StderrLevel {
    Error,
    Warn,
    Info,
    Debug,

    Unknown,
}

#[must_use]
pub fn stderr_level(line: &str) -> StderrLevel {
    let mut tokens = line.split_whitespace();
    let Some(first) = tokens.next() else {
        return StderrLevel::Unknown;
    };
    let is_uptime = first
        .strip_suffix('s')
        .is_some_and(|n| !n.is_empty() && n.parse::<f64>().is_ok());
    let token = if is_uptime {
        tokens.next()
    } else {
        Some(first)
    };
    match token.map(str::to_ascii_uppercase).as_deref() {
        Some("ERROR") => StderrLevel::Error,
        Some("WARN" | "WARNING") => StderrLevel::Warn,
        Some("INFO") => StderrLevel::Info,
        Some("DEBUG" | "TRACE") => StderrLevel::Debug,
        _ => StderrLevel::Unknown,
    }
}

#[cfg(test)]
mod tests;
