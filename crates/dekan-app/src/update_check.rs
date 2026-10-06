use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dekan_core::phase::GamePhase;
use dekan_core::state::StateReceiver;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

const USER_AGENT: &str = concat!("Dekan/", env!("CARGO_PKG_VERSION"), " (update check)");
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(45);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const BUSY_RETRY: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const NOTIFIED_FILE: &str = "update_notified.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    #[must_use]
    pub fn parse(tag: &str) -> Option<Self> {
        let raw = tag.trim();
        let raw = raw
            .strip_prefix('v')
            .or_else(|| raw.strip_prefix('V'))
            .unwrap_or(raw);
        let mut parts = raw.split('.');
        let mut next = |required: bool| match parts.next() {
            Some(part) if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) => {
                part.parse::<u64>().ok()
            }
            None if !required => Some(0),
            _ => None,
        };
        let major = next(true)?;
        let minor = next(true)?;
        let patch = next(false)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    #[must_use]
    pub fn current() -> Option<Self> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.patch == 0 {
            write!(f, "{}.{}", self.major, self.minor)
        } else {
            write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
        }
    }
}

#[derive(Debug, Deserialize)]
struct LatestRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

pub fn newer_release(current: Version, body: &str) -> Result<Option<Version>, String> {
    let release: LatestRelease =
        serde_json::from_str(body).map_err(|e| format!("unreadable release data: {e}"))?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let latest = Version::parse(&release.tag_name)
        .ok_or_else(|| format!("release tag '{}' is not a version", release.tag_name))?;
    Ok((latest > current).then_some(latest))
}

#[must_use]
pub fn latest_release_api() -> String {
    let repo = REPOSITORY.trim_end_matches('/');
    let path = repo.strip_prefix("https://github.com/").unwrap_or(repo);
    format!("https://api.github.com/repos/{path}/releases/latest")
}

#[must_use]
pub fn release_page() -> String {
    format!("{}/releases/latest", REPOSITORY.trim_end_matches('/'))
}

#[must_use]
pub fn is_enabled(env_value: Option<&str>) -> bool {
    !matches!(
        env_value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0" | "off" | "false" | "no")
    )
}

#[must_use]
pub fn is_busy(phase: GamePhase) -> bool {
    matches!(
        phase,
        GamePhase::ReadyCheck
            | GamePhase::ChampSelect
            | GamePhase::Finalization
            | GamePhase::GameStart
            | GamePhase::InProgress
            | GamePhase::Reconnect
    )
}

#[derive(Debug, Clone, Default)]
pub struct UpdateNotice(Arc<Mutex<Option<Version>>>);

impl UpdateNotice {
    #[must_use]
    pub fn available(&self) -> Option<Version> {
        self.0.lock().map(|v| *v).unwrap_or_default()
    }

    fn set(&self, version: Version) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(version);
        }
    }
}

#[must_use]
pub fn already_notified(state_dir: &Path, version: Version) -> bool {
    std::fs::read_to_string(state_dir.join(NOTIFIED_FILE))
        .is_ok_and(|saved| saved.trim() == version.to_string())
}

pub fn remember_notified(state_dir: &Path, version: Version) {
    if let Err(e) = dekan_platform::fs::atomic_write(
        &state_dir.join(NOTIFIED_FILE),
        version.to_string().as_bytes(),
        false,
    ) {
        warn!(error = %e, version = %version, "Could not record the update notice; it may be shown again next launch");
    }
}

pub struct UpdateCheck {
    pub state_dir: PathBuf,
    pub notice: UpdateNotice,
    pub notify: Box<dyn Fn(Version) + Send>,
}

async fn fetch_latest(
    client: &reqwest::Client,
    current: Version,
) -> Result<Option<Version>, String> {
    let response = client
        .get(latest_release_api())
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("GitHub answered HTTP {}", response.status()));
    }
    let body = response.text().await.map_err(|e| e.to_string())?;
    newer_release(current, &body)
}

async fn pause(token: &CancellationToken, duration: Duration) -> bool {
    tokio::select! {
        _ = token.cancelled() => false,
        () = tokio::time::sleep(duration) => true,
    }
}

pub async fn run(check: UpdateCheck, state_rx: StateReceiver, token: CancellationToken) {
    let Some(current) = Version::current() else {
        warn!("Update check off: the running version could not be read");
        return;
    };
    let client = match reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build() {
        Ok(client) => client,
        Err(e) => {
            warn!(error = %e, "Update check off: the HTTP client could not be created");
            return;
        }
    };
    if !pause(&token, FIRST_CHECK_DELAY).await {
        return;
    }
    loop {
        match fetch_latest(&client, current).await {
            Ok(Some(latest)) => {
                if check.notice.available() != Some(latest) {
                    info!(current = %current, latest = %latest, "A newer Dekan release is available");
                    check.notice.set(latest);
                }
                if !already_notified(&check.state_dir, latest) {
                    while is_busy(state_rx.borrow().phase) {
                        if !pause(&token, BUSY_RETRY).await {
                            return;
                        }
                    }
                    (check.notify)(latest);
                    remember_notified(&check.state_dir, latest);
                }
            }
            Ok(None) => debug!(current = %current, "Dekan is up to date"),
            Err(e) => debug!(error = %e, "Update check failed; trying again later"),
        }
        if !pause(&token, CHECK_INTERVAL).await {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(tag: &str) -> Version {
        Version::parse(tag).expect("version")
    }

    #[test]
    fn tags_in_every_released_shape_parse() {
        assert_eq!(v("v1.1"), v("1.1.0"));
        assert_eq!(v("V2.0"), v("2.0.0"));
        assert_eq!(v("1.0.0"), v("v1.0"));
        assert_eq!(v("v1.2.3").to_string(), "1.2.3");
        assert_eq!(v("v1.10").to_string(), "1.10");
    }

    #[test]
    fn anything_but_a_plain_version_is_refused() {
        for tag in [
            "", "v", "1", "v1.", "1.1-beta", "1.1.0.4", "latest", "1.x", "+1.2", "1. 2",
        ] {
            assert_eq!(Version::parse(tag), None, "{tag}");
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(v("1.10") > v("1.9"));
        assert!(v("2.0") > v("1.99.99"));
        assert!(v("1.1.1") > v("1.1"));
    }

    #[test]
    fn the_running_version_is_readable() {
        assert!(Version::current().is_some());
    }

    #[test]
    fn only_a_newer_published_release_is_offered() {
        let current = v("1.1");
        let newer = r#"{"tag_name":"v1.2","draft":false,"prerelease":false,"html_url":"x"}"#;
        assert_eq!(newer_release(current, newer), Ok(Some(v("1.2"))));
        let same = r#"{"tag_name":"v1.1"}"#;
        assert_eq!(newer_release(current, same), Ok(None));
        let older = r#"{"tag_name":"v1.0.0"}"#;
        assert_eq!(newer_release(current, older), Ok(None));
        let pre = r#"{"tag_name":"v9.0","prerelease":true}"#;
        assert_eq!(newer_release(current, pre), Ok(None));
        let draft = r#"{"tag_name":"v9.0","draft":true}"#;
        assert_eq!(newer_release(current, draft), Ok(None));
    }

    #[test]
    fn malformed_answers_are_errors_not_offers() {
        let current = v("1.1");
        assert!(newer_release(current, "not json").is_err());
        assert!(newer_release(current, r#"{"message":"Not Found"}"#).is_err());
        assert!(newer_release(current, r#"{"tag_name":"nightly"}"#).is_err());
    }

    #[test]
    fn urls_point_at_the_project_repository() {
        let repo = REPOSITORY.trim_end_matches('/');
        let path = repo.trim_start_matches("https://github.com/");
        assert!(repo.starts_with("https://github.com/"));
        assert_eq!(
            latest_release_api(),
            format!("https://api.github.com/repos/{path}/releases/latest")
        );
        assert_eq!(release_page(), format!("{repo}/releases/latest"));
    }

    #[test]
    fn the_check_can_be_turned_off() {
        assert!(is_enabled(None));
        assert!(is_enabled(Some("1")));
        assert!(is_enabled(Some("on")));
        for off in ["0", "off", "OFF", " false ", "no"] {
            assert!(!is_enabled(Some(off)), "{off}");
        }
    }

    #[test]
    fn no_notice_interrupts_a_match() {
        for phase in [
            GamePhase::ReadyCheck,
            GamePhase::ChampSelect,
            GamePhase::Finalization,
            GamePhase::GameStart,
            GamePhase::InProgress,
            GamePhase::Reconnect,
        ] {
            assert!(is_busy(phase), "{phase:?}");
        }
        for phase in [GamePhase::None, GamePhase::Lobby, GamePhase::EndOfGame] {
            assert!(!is_busy(phase), "{phase:?}");
        }
    }

    #[test]
    fn a_version_is_announced_once() {
        let dir = std::env::temp_dir().join(format!("dekan_update_notice_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&dir).expect("fixture dir");
        assert!(!already_notified(&dir, v("1.2")));
        remember_notified(&dir, v("1.2"));
        assert!(already_notified(&dir, v("1.2")));
        assert!(!already_notified(&dir, v("1.3")));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
    }

    #[test]
    fn the_notice_holds_the_latest_seen_version() {
        let notice = UpdateNotice::default();
        assert_eq!(notice.available(), None);
        notice.set(v("1.2"));
        assert_eq!(notice.clone().available(), Some(v("1.2")));
    }
}
