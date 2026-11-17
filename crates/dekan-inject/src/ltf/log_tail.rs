use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerLogEvent {
    OverlayBuildComplete(String),
    ScanningStarted(String),
    GameFound {
        tid: Option<u32>,
        pid: Option<u32>,
        line: String,
    },
    DllAttached {
        pid: Option<u32>,
        line: String,
    },
    DllInitDone(String),
    RedirectedWad(String),
    JoinedTooLate(String),
    HookFailed(String),
    VerificationWarning(String),
    VerificationFailed(String),
    ElevationMismatch(String),
    StartFailed(String),
    PatcherStopped(String),
}

pub struct ManagerLogTail {
    logs_dir: Option<PathBuf>,
    current_file: Option<PathBuf>,
    offset: u64,
    partial_line: String,
}

impl ManagerLogTail {
    #[must_use]
    pub fn from_default_dir_at_end() -> Self {
        Self::new_at_end(default_manager_logs_dir())
    }

    #[must_use]
    pub fn new_at_end(logs_dir: Option<PathBuf>) -> Self {
        let current_file = logs_dir.as_deref().and_then(find_newest_log);
        let offset = current_file
            .as_deref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map_or(0, |m| m.len());
        Self {
            logs_dir,
            current_file,
            offset,
            partial_line: String::new(),
        }
    }

    pub fn poll_events(&mut self) -> Vec<ManagerLogEvent> {
        let Some(ref dir) = self.logs_dir else {
            return Vec::new();
        };
        if let Some(latest) = find_newest_log(dir) {
            if self.current_file.as_ref() != Some(&latest) {
                self.current_file = Some(latest);
                self.offset = 0;
                self.partial_line.clear();
            }
        }
        let Some(ref path) = self.current_file else {
            return Vec::new();
        };
        let Ok(mut file) = File::open(path) else {
            return Vec::new();
        };
        let Ok(len) = file.metadata().map(|m| m.len()) else {
            return Vec::new();
        };
        if len < self.offset {
            self.offset = 0;
            self.partial_line.clear();
        }
        if len == self.offset {
            return Vec::new();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut buf = Vec::new();
        if file.read_to_end(&mut buf).is_err() {
            return Vec::new();
        }
        self.offset += buf.len() as u64;
        self.partial_line.push_str(&String::from_utf8_lossy(&buf));

        let mut events = Vec::new();
        while let Some(pos) = self.partial_line.find('\n') {
            let line = self.partial_line[..pos].trim_end_matches('\r').trim();
            if let Some(ev) = parse_manager_log_line(line) {
                events.push(ev);
            }
            self.partial_line.drain(..=pos);
        }
        events
    }
}

#[must_use]
pub fn default_manager_logs_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("dev.leaguetoolkit.manager").join("logs"))
}

fn find_newest_log(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(String, std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("ltk-manager") || !name.ends_with(".log") {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        let name_owned = name.to_string();
        let replace = match best {
            None => true,
            Some((ref prev_name, prev_time, _)) => {
                (name_owned.as_str(), mtime) > (prev_name.as_str(), prev_time)
            }
        };
        if replace {
            best = Some((name_owned, mtime, path));
        }
    }
    best.map(|(_, _, path)| path)
}

#[must_use]
pub fn parse_manager_log_line(raw: &str) -> Option<ManagerLogEvent> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }
    let lower = line.to_ascii_lowercase();

    if lower.contains("joined too late") {
        return Some(ManagerLogEvent::JoinedTooLate(line.to_owned()));
    }
    if lower.contains("lazy verification failed")
        || lower.contains("integrity scan rejected")
        || lower.contains("wad scan failed")
        || contains_vrf_code(line)
    {
        return Some(ManagerLogEvent::VerificationFailed(line.to_owned()));
    }
    if lower.contains("base-skin check:") {
        return Some(ManagerLogEvent::VerificationWarning(line.to_owned()));
    }
    if lower.contains("hook install failed") || lower.contains("setwindowshookex failed") {
        return Some(ManagerLogEvent::HookFailed(line.to_owned()));
    }
    if lower.contains("os error 740")
        || lower.contains("manager_elevated=true")
        || lower.contains("league_admin=true")
    {
        return Some(ManagerLogEvent::ElevationMismatch(line.to_owned()));
    }
    if lower.contains("start patcher failed") {
        return Some(ManagerLogEvent::StartFailed(line.to_owned()));
    }
    if lower.contains("game found; hook installed") {
        return Some(ManagerLogEvent::GameFound {
            tid: extract_kv_u32(line, "tid="),
            pid: extract_kv_u32(line, "pid="),
            line: line.to_owned(),
        });
    }
    if lower.contains("dll attached") {
        return Some(ManagerLogEvent::DllAttached {
            pid: extract_kv_u32(line, "pid="),
            line: line.to_owned(),
        });
    }
    if lower.contains("ltk_patcher_dll::entry: init done") {
        return Some(ManagerLogEvent::DllInitDone(line.to_owned()));
    }
    if lower.contains("redirected wad:") {
        return Some(ManagerLogEvent::RedirectedWad(line.to_owned()));
    }
    if lower.contains("session started: scanning for game")
        || lower.contains("[ltk-host] injecting: scanning for game")
    {
        return Some(ManagerLogEvent::ScanningStarted(line.to_owned()));
    }
    if lower.contains("overlay build complete:") {
        return Some(ManagerLogEvent::OverlayBuildComplete(line.to_owned()));
    }
    if lower.contains("patcher thread exiting") || lower.contains("injector stopped") {
        return Some(ManagerLogEvent::PatcherStopped(line.to_owned()));
    }
    None
}

fn contains_vrf_code(line: &str) -> bool {
    let bytes = line.as_bytes();
    if bytes.len() < 11 {
        return false;
    }
    bytes.windows(11).any(|w| {
        w[0] == b'V'
            && w[1] == b'R'
            && w[2] == b'F'
            && w[3..11].iter().all(|b| b.is_ascii_hexdigit())
    })
}

fn extract_kv_u32(line: &str, prefix: &str) -> Option<u32> {
    let pos = line.find(prefix)?;
    let digits: String = line[pos + prefix.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn parses_real_manager_log_events() {
        assert!(matches!(
            parse_manager_log_line(
                "2026-11-16T23:22:00.309654Z  WARN ltk_manager: game found; hook installed tid=3764 pid=25932"
            ),
            Some(ManagerLogEvent::GameFound {
                tid: Some(3764),
                pid: Some(25932),
                ..
            })
        ));
        assert!(matches!(
            parse_manager_log_line(
                "2026-11-16T23:22:00.407922Z  WARN ltk_manager: dll attached pid=25932"
            ),
            Some(ManagerLogEvent::DllAttached {
                pid: Some(25932),
                ..
            })
        ));
        assert!(matches!(
            parse_manager_log_line(
                "2026-11-16T23:18:26.879022Z INFO ltk_patcher_dll::entry: joined too late, not overlaying"
            ),
            Some(ManagerLogEvent::JoinedTooLate(_))
        ));
        assert!(matches!(
            parse_manager_log_line(
                "2026-11-16T23:22:03.670560Z INFO ltk_patcher_dll::verify: lazy verification failed, not overlaying: base-skin check"
            ),
            Some(ManagerLogEvent::VerificationFailed(_))
        ));
        assert!(matches!(
            parse_manager_log_line(
                "2026-11-17T01:09:45.146080Z WARN ltk_patcher_dll::verify: base-skin check: skin database for yasuo is from another game version"
            ),
            Some(ManagerLogEvent::VerificationWarning(_))
        ));
        assert!(matches!(
            parse_manager_log_line("host refused [VRF00000007]"),
            Some(ManagerLogEvent::VerificationFailed(_))
        ));
        assert!(matches!(
            parse_manager_log_line("SetWindowsHookEx failed: 0x5"),
            Some(ManagerLogEvent::HookFailed(_))
        ));
    }

    #[test]
    fn log_tail_reads_increments_and_follows_date_rotation() {
        let dir = std::env::temp_dir().join(format!("dekan_manager_tail_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: clean prior test dir
        std::fs::create_dir_all(&dir).expect("mkdir");

        let day1 = dir.join("ltk-manager.2026-11-16.log");
        std::fs::write(&day1, "old line\nsession started: scanning for game\n")
            .expect("write day1");

        let mut tail = ManagerLogTail::new_at_end(Some(dir.clone()));
        assert!(tail.poll_events().is_empty());

        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&day1)
            .expect("open append");
        writeln!(f, "game found; hook installed tid=111 pid=222").expect("append 1");
        let evs = tail.poll_events();
        assert_eq!(evs.len(), 1);
        assert!(matches!(
            evs[0],
            ManagerLogEvent::GameFound {
                tid: Some(111),
                pid: Some(222),
                ..
            }
        ));

        let day2 = dir.join("ltk-manager.2026-11-17.log");
        std::fs::write(&day2, "dll attached pid=333\n").expect("write day2");
        let evs2 = tail.poll_events();
        assert_eq!(evs2.len(), 1);
        assert!(matches!(
            evs2[0],
            ManagerLogEvent::DllAttached { pid: Some(333), .. }
        ));

        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup test dir
    }
}
