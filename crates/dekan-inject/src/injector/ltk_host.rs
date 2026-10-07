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
mod tests {
    use super::*;

    #[test]
    fn test_dll_support_is_decided_by_the_game_build_not_the_clock() {
        let limit = 0x6ad4_6e70;
        let far = u64::from(limit) - 30 * 86_400;

        assert_eq!(
            dll_support(limit - 12 * 86_400, limit, far),
            DllSupport::Supported
        );

        assert_eq!(
            dll_support(limit - 12 * 86_400, limit, u64::from(limit) - 3 * 86_400),
            DllSupport::SupportedUntilNextPatch { days_left: 3 }
        );

        assert!(matches!(
            dll_support(limit - 12 * 86_400, limit, u64::from(limit) + 10 * 86_400),
            DllSupport::SupportedUntilNextPatch { days_left } if days_left < 0
        ));

        assert_ne!(dll_support(limit, limit, far), DllSupport::Refused);
        assert_eq!(dll_support(limit + 1, limit, far), DllSupport::Refused);
    }

    #[test]
    fn test_stderr_level_reads_the_hosts_own_level() {
        let real = [
            (
                "    0.000037900s  INFO ltk_patcher_host: host starting (normal mode)",
                StderrLevel::Info,
            ),
            (
                "    0.002352700s  INFO ltk_patcher_host::worker: session started: scanning for game",
                StderrLevel::Info,
            ),
            (
                "    0.052160100s  INFO ltk_patcher_host::worker: game found; hook installed tid=7872 pid=14920",
                StderrLevel::Info,
            ),
            (
                "    0.719805800s  INFO ltk_patcher_host::worker: dll attached pid=12936",
                StderrLevel::Info,
            ),
            (
                "    1.200000000s ERROR ltk_patcher_host::worker: hook install failed tid=1 pid=2 error=0",
                StderrLevel::Error,
            ),
            (
                "    0.1s  WARN ltk_patcher_host: slow scan",
                StderrLevel::Warn,
            ),
            ("DEBUG ltk_patcher_host: detail", StderrLevel::Debug),
            ("   2.5s TRACE x: y", StderrLevel::Debug),
        ];
        for (line, level) in real {
            assert_eq!(stderr_level(line), level, "{line}");
        }
    }

    #[test]
    fn test_stderr_level_unknown_is_never_mistaken_for_harmless() {
        for line in [
            "",
            "   ",
            "thread 'main' panicked at src/main.rs:1:1",
            "s INFO not an uptime",
            "0.1s",
            "12.0x INFO wrong unit",
        ] {
            assert_eq!(stderr_level(line), StderrLevel::Unknown, "{line:?}");
        }
    }

    #[test]
    fn test_patcher_flags_override_and_bad_value() {
        assert_eq!(flags_from(None), hook_flags::OPT_OUT_AH_V1);
        assert_eq!(flags_from(Some(" 0 ")), 0);
        assert_eq!(flags_from(Some("12")), 12);
        assert_eq!(flags_from(Some("four")), hook_flags::OPT_OUT_AH_V1);
        assert_eq!(flags_from(Some("-1")), hook_flags::OPT_OUT_AH_V1);
    }

    #[test]
    fn status_injected_is_the_confirmation() {
        assert_eq!(
            parse_host_event("status 0.12 injected dll attached"),
            Some(HostEvent::Status {
                state: HostState::Injected,
                message: "dll attached".into(),
            })
        );
    }

    #[test]
    fn status_failed_keeps_the_reason() {
        let ev = parse_host_event(
            "status 60.0 failed DLL never attached after 60s -- check the DLL signature / antivirus",
        )
        .expect("status");
        match ev {
            HostEvent::Status { state, message } => {
                assert_eq!(state, HostState::Failed);
                assert!(message.contains("DLL never attached"));
            }
            _ => panic!("expected status"),
        }
    }

    #[test]
    fn dll_log_splits_level_from_message() {
        let ev = parse_host_event(
            "dll 10.1 1234 5678 ERROR ltk_patcher_dll::verify: WAD scan failed status with c0000229 for briar.wad.client",
        )
        .expect("dll");
        match ev {
            HostEvent::DllLog { level, message } => {
                assert_eq!(level, "ERROR");
                assert!(message.contains("WAD scan failed status with c0000229"));
                assert!(is_expected_status(&message));

                assert!(!is_dll_failure(&level, &message));
            }
            _ => panic!("expected dll log"),
        }

        assert!(is_dll_failure("ERROR", "failed to patch CreateFileA"));
    }

    #[test]
    fn a_redirected_archive_is_named() {
        assert_eq!(
            redirected_wad(
                "ltk_patcher_dll::hooks::fsov::imp: redirected wad: DATA/FINAL/Maps/Shipping/Map11.wad.client"
            ),
            Some("DATA/FINAL/Maps/Shipping/Map11.wad.client")
        );
        assert_eq!(redirected_wad("ltk_patcher_dll::entry: init done"), None);
        assert_eq!(redirected_wad("redirected wad:   "), None);
    }

    #[test]
    fn end_of_life_is_detected_and_is_a_failure() {
        let msg = "ltk_patcher_dll::entry: end of life reached, please update: 0x6ac1f970";
        assert!(is_end_of_life(msg));
        assert!(is_dll_failure("info", msg));
    }

    #[test]
    fn progress_chatter_is_not_a_failure() {
        assert!(!is_dll_failure(
            "info",
            "redirected wad: DATA/FINAL/Champions/Zed.wad.client"
        ));
        assert!(!is_dll_failure("info", "init done"));
    }

    #[test]
    fn blank_and_unknown_lines_are_dropped() {
        assert_eq!(parse_host_event(""), None);
        assert_eq!(parse_host_event("\r\n"), None);
        assert_eq!(parse_host_event("foobar 1.0 x"), None);
        assert_eq!(parse_host_event("status 1.0 martian x"), None);
    }

    #[test]
    fn default_flags_opt_out_of_ah_by_default() {
        assert_eq!(hook_flags::OPT_OUT_AH_V1, 4);
    }
}
