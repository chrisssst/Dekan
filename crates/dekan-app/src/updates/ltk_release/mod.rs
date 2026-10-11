use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dekan_core::state::StateReceiver;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::update_check::{Version, is_busy};

pub const LTK_REPOSITORY: &str = "https://github.com/LeagueToolkit/ltk-manager";

const LTK_API: &str = "https://api.github.com/repos/LeagueToolkit/ltk-manager";
const RESOURCES_PATH: &str = "src-tauri/resources";
const USER_AGENT: &str = concat!("Dekan/", env!("CARGO_PKG_VERSION"), " (injector check)");
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const BUSY_RETRY: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BINARY_BYTES: usize = 16 * 1024 * 1024;
const RELEASES_PER_PAGE: u32 = 30;
const MAX_INSPECTIONS_PER_CHECK: usize = 12;
const VERDICTS_FILE: &str = "ltk_releases.txt";
const NOTIFIED_FILE: &str = "ltk_notified.txt";

pub const INJECTOR_FILES: [&str; 2] = [
    dekan_inject::ltk_host::HOST_EXE,
    dekan_inject::ltk_host::DLL_FILE,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Injector {
    Trusted { dll_sha256: String },
    Untrusted,
}

impl Injector {
    fn render(&self) -> String {
        match self {
            Self::Trusted { dll_sha256 } => format!("trusted {dll_sha256}"),
            Self::Untrusted => "untrusted".to_owned(),
        }
    }

    fn parse(words: &[&str]) -> Option<Self> {
        match words {
            ["trusted", dll] if dll.len() == 64 && dll.chars().all(|c| c.is_ascii_hexdigit()) => {
                Some(Self::Trusted {
                    dll_sha256: dll.to_ascii_lowercase(),
                })
            }
            ["untrusted"] => Some(Self::Untrusted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LtkStatus {
    pub latest: String,
    pub latest_trusted: bool,
    pub compatible: Option<String>,
    pub compatible_dll: Option<String>,
}

impl LtkStatus {
    #[must_use]
    pub fn offers_update_over(&self, installed_dll_sha256: Option<&str>) -> Option<&str> {
        let dll = self.compatible_dll.as_deref()?;
        let installed = installed_dll_sha256.unwrap_or_default();
        (!dll.eq_ignore_ascii_case(installed))
            .then_some(self.compatible.as_deref())
            .flatten()
    }
}

#[must_use]
pub fn release_page(version: Option<&str>) -> String {
    match version {
        Some(version) => format!("{LTK_REPOSITORY}/releases/tag/v{version}"),
        None => format!("{LTK_REPOSITORY}/releases"),
    }
}

#[derive(Debug, Deserialize)]
struct PublishedRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

pub fn published_versions(body: &str) -> Result<Vec<String>, String> {
    let releases: Vec<PublishedRelease> =
        serde_json::from_str(body).map_err(|e| format!("unreadable release list: {e}"))?;
    let mut versions: Vec<(Version, String)> = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let tag = r.tag_name.trim();
            let raw = tag
                .strip_prefix('v')
                .or_else(|| tag.strip_prefix('V'))
                .unwrap_or(tag);
            Version::parse(raw).map(|v| (v, raw.to_owned()))
        })
        .collect();
    versions.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    versions.dedup_by(|a, b| a.0 == b.0);
    Ok(versions.into_iter().map(|(_, raw)| raw).collect())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdicts {
    by_version: BTreeMap<String, Injector>,
}

impl Verdicts {
    #[must_use]
    pub fn get(&self, version: &str) -> Option<&Injector> {
        self.by_version.get(version)
    }

    pub fn insert(&mut self, version: &str, injector: Injector) {
        self.by_version.insert(version.to_owned(), injector);
    }

    #[must_use]
    pub fn status(&self, newest_first: &[String]) -> Option<LtkStatus> {
        let latest = newest_first.first()?;
        let latest_trusted = matches!(self.get(latest)?, Injector::Trusted { .. });
        let compatible = newest_first.iter().find_map(|v| match self.get(v) {
            Some(Injector::Trusted { dll_sha256 }) => Some((v.clone(), dll_sha256.clone())),
            _ => None,
        });
        Some(LtkStatus {
            latest: latest.clone(),
            latest_trusted,
            compatible_dll: compatible.as_ref().map(|(_, dll)| dll.clone()),
            compatible: compatible.map(|(version, _)| version),
        })
    }

    #[must_use]
    pub fn status_from_cache(&self) -> Option<LtkStatus> {
        let mut known: Vec<(Version, &String)> = self
            .by_version
            .keys()
            .filter_map(|raw| Version::parse(raw).map(|v| (v, raw)))
            .collect();
        known.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        let ordered: Vec<String> = known.into_iter().map(|(_, raw)| raw.clone()).collect();
        self.status(&ordered)
    }

    fn render(&self) -> String {
        let mut out = format!("{}\n", fingerprint());
        for (version, injector) in &self.by_version {
            out.push_str(&format!("{version} {}\n", injector.render()));
        }
        out
    }

    fn parse(raw: &str) -> Self {
        let mut lines = raw.lines();
        let mut verdicts = Self::default();
        if lines.next().map(str::trim) != Some(fingerprint().as_str()) {
            return verdicts;
        }
        for line in lines {
            let words: Vec<&str> = line.split_whitespace().collect();
            if let [version, rest @ ..] = words.as_slice() {
                if let (Some(_), Some(injector)) = (Version::parse(version), Injector::parse(rest))
                {
                    verdicts.insert(version, injector);
                }
            }
        }
        verdicts
    }
}

fn fingerprint() -> String {
    format!("signed-by {}", dekan_inject::trust::LTK_PUBLISHER)
}

#[must_use]
pub fn load_verdicts(state_dir: &Path) -> Verdicts {
    std::fs::read_to_string(state_dir.join(VERDICTS_FILE))
        .map(|raw| Verdicts::parse(&raw))
        .unwrap_or_default()
}

pub fn save_verdicts(state_dir: &Path, verdicts: &Verdicts) {
    if let Err(e) = dekan_platform::fs::atomic_write(
        &state_dir.join(VERDICTS_FILE),
        verdicts.render().as_bytes(),
        false,
    ) {
        warn!(error = %e, "Could not record the LTK Manager release check; it will run again next launch");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledDll {
    pub sha256: String,
    pub build_limit: Option<u32>,
}

impl InstalledDll {
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            sha256: dekan_inject::dll_validator::compute_sha256(bytes),
            build_limit: dekan_inject::trust::dll_build_limit(bytes),
        }
    }
}

type DllStamp = (std::time::SystemTime, u64);

#[derive(Debug, Clone, Default)]
pub struct InstalledDllCache(Arc<Mutex<Option<(DllStamp, InstalledDll)>>>);

impl InstalledDllCache {
    #[must_use]
    pub fn read(&self, dll: &Path) -> Option<InstalledDll> {
        let meta = std::fs::metadata(dll).ok()?;
        let stamp = (meta.modified().ok()?, meta.len());
        if let Ok(slot) = self.0.lock() {
            if let Some((known, installed)) = slot.as_ref() {
                if *known == stamp {
                    return Some(installed.clone());
                }
            }
        }
        let installed = InstalledDll::from_bytes(&std::fs::read(dll).ok()?);
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some((stamp, installed.clone()));
        }
        Some(installed)
    }
}

#[must_use]
pub fn already_notified(state_dir: &Path, version: &str) -> bool {
    std::fs::read_to_string(state_dir.join(NOTIFIED_FILE))
        .is_ok_and(|saved| saved.trim() == version)
}

pub fn remember_notified(state_dir: &Path, version: &str) {
    if let Err(e) =
        dekan_platform::fs::atomic_write(&state_dir.join(NOTIFIED_FILE), version.as_bytes(), false)
    {
        warn!(error = %e, version = %version, "Could not record the injector notice; it may be shown again next launch");
    }
}

#[derive(Debug, Clone, Default)]
pub struct LtkNotice(Arc<Mutex<Option<LtkStatus>>>);

impl LtkNotice {
    #[must_use]
    pub fn status(&self) -> Option<LtkStatus> {
        self.0.lock().map(|v| v.clone()).unwrap_or_default()
    }

    fn set(&self, status: LtkStatus) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(status);
        }
    }
}

pub struct LtkCheck {
    pub state_dir: PathBuf,
    pub installed_dll: PathBuf,
    pub installed: InstalledDllCache,
    pub notice: LtkNotice,
    pub notify: Box<dyn Fn(&str) + Send>,
}

async fn pause(token: &CancellationToken, duration: Duration) -> bool {
    tokio::select! {
        _ = token.cancelled() => false,
        () = tokio::time::sleep(duration) => true,
    }
}

pub async fn run(check: LtkCheck, state_rx: StateReceiver, token: CancellationToken) {
    let client = match http_client() {
        Ok(client) => client,
        Err(e) => {
            warn!(error = %e, "Injector check off: the HTTP client could not be created");
            return;
        }
    };
    let mut verdicts = load_verdicts(&check.state_dir);
    if let Some(status) = verdicts.status_from_cache() {
        check.notice.set(status);
    }
    if !pause(&token, FIRST_CHECK_DELAY).await {
        return;
    }
    loop {
        match refresh(&client, &mut verdicts).await {
            Ok(Some(status)) => {
                save_verdicts(&check.state_dir, &verdicts);
                if check.notice.status().as_ref() != Some(&status) {
                    info!(
                        latest = %status.latest,
                        latest_trusted = status.latest_trusted,
                        compatible = status.compatible.as_deref().unwrap_or("none"),
                        "LTK Manager releases checked against the publisher's signature"
                    );
                    check.notice.set(status.clone());
                }
                let installed = check
                    .installed
                    .read(&check.installed_dll)
                    .map(|dll| dll.sha256);
                if let Some(version) = status.offers_update_over(installed.as_deref()) {
                    if !already_notified(&check.state_dir, version) {
                        while is_busy(state_rx.borrow().phase) {
                            if !pause(&token, BUSY_RETRY).await {
                                return;
                            }
                        }
                        (check.notify)(version);
                        remember_notified(&check.state_dir, version);
                    }
                }
            }
            Ok(None) => {
                save_verdicts(&check.state_dir, &verdicts);
                debug!("No published LTK Manager release could be classified");
            }
            Err(e) => {
                save_verdicts(&check.state_dir, &verdicts);
                debug!(error = %e, "Injector check failed; trying again later");
            }
        }
        if !pause(&token, CHECK_INTERVAL).await {
            return;
        }
    }
}

mod fetch;

pub use fetch::{compatible_version, download_injector};
use fetch::{http_client, refresh};

#[cfg(test)]
mod tests;
