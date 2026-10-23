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
const CHECK_INTERVAL: Duration = Duration::from_secs(30 * 60);
const BUSY_RETRY: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BINARY_BYTES: usize = 16 * 1024 * 1024;
const RELEASES_PER_PAGE: u32 = 30;
const MAX_INSPECTIONS_PER_CHECK: usize = 12;
const VERDICTS_FILE: &str = "ltk_releases.txt";
const NOTIFIED_FILE: &str = "ltk_notified.txt";
const NOT_FOUND_MARK: &str = "HTTP 404";

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
    let mut unreadable = Vec::new();
    let mut versions: Vec<(Version, String)> = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let tag = r.tag_name.trim();
            let raw = tag
                .strip_prefix('v')
                .or_else(|| tag.strip_prefix('V'))
                .unwrap_or(tag);
            let parsed = Version::parse(raw).map(|v| (v, raw.to_owned()));
            if parsed.is_none() {
                unreadable.push(tag.to_owned());
            }
            parsed
        })
        .collect();
    if let Some(first) = unreadable.first() {
        if versions.is_empty() {
            warn!(dropped = unreadable.len(), first_tag = %first, "No LTK Manager release tag is a readable version");
        } else {
            debug!(dropped = unreadable.len(), first_tag = %first, "LTK Manager release tags that are not versions were skipped");
        }
    }
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
            debug!(
                "LTK release verdicts were recorded under another publisher key or format; they are checked again"
            );
            return verdicts;
        }
        let mut dropped = 0usize;
        for line in lines {
            let words: Vec<&str> = line.split_whitespace().collect();
            if let [version, rest @ ..] = words.as_slice() {
                match (Version::parse(version), Injector::parse(rest)) {
                    (Some(_), Some(injector)) => verdicts.insert(version, injector),
                    _ => dropped += 1,
                }
            }
        }
        if dropped > 0 {
            debug!(
                dropped,
                "Unreadable lines of the LTK release verdicts were dropped"
            );
        }
        verdicts
    }
}

fn fingerprint() -> String {
    format!(
        "signed-by {} or unsigned, any host",
        dekan_inject::trust::LTK_PUBLISHER
    )
}

#[must_use]
pub fn load_verdicts(state_dir: &Path) -> Verdicts {
    match std::fs::read_to_string(state_dir.join(VERDICTS_FILE)) {
        Ok(raw) => Verdicts::parse(&raw),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(error = %e, "Could not read the LTK Manager release record; the releases are checked again");
            }
            Verdicts::default()
        }
    }
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

fn log_unreadable_dll(dll: &Path, e: &std::io::Error) {
    if e.kind() != std::io::ErrorKind::NotFound {
        debug!(file = %dll.display(), error = %e, "The installed patcher DLL could not be inspected");
    }
}

#[derive(Debug, Clone, Default)]
pub struct InstalledDllCache(Arc<Mutex<Option<(DllStamp, InstalledDll)>>>);

impl InstalledDllCache {
    #[must_use]
    pub fn read(&self, dll: &Path) -> Option<InstalledDll> {
        let meta = std::fs::metadata(dll)
            .inspect_err(|e| log_unreadable_dll(dll, e))
            .ok()?;
        let stamp = (
            meta.modified()
                .inspect_err(|e| log_unreadable_dll(dll, e))
                .ok()?,
            meta.len(),
        );
        if let Ok(slot) = self.0.lock() {
            if let Some((known, installed)) = slot.as_ref() {
                if *known == stamp {
                    return Some(installed.clone());
                }
            }
        }
        let bytes = std::fs::read(dll)
            .inspect_err(|e| log_unreadable_dll(dll, e))
            .ok()?;
        let installed = InstalledDll::from_bytes(&bytes);
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some((stamp, installed.clone()));
        }
        Some(installed)
    }
}

#[must_use]
pub fn already_notified(state_dir: &Path, version: &str) -> bool {
    match std::fs::read_to_string(state_dir.join(NOTIFIED_FILE)) {
        Ok(saved) => saved.trim() == version,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(error = %e, "Could not read the injector notice record; the notice may repeat");
            }
            false
        }
    }
}

pub fn remember_notified(state_dir: &Path, version: &str) {
    if let Err(e) =
        dekan_platform::fs::atomic_write(&state_dir.join(NOTIFIED_FILE), version.as_bytes(), false)
    {
        warn!(error = %e, version = %version, "Could not record the injector notice; it may be shown again next launch");
    }
}

mod fetch;
mod watch;

pub use fetch::{compatible_version, download_injector, is_release_version};
use fetch::{http_client, refresh};
pub use watch::{LtkCheck, LtkNotice, run};

#[cfg(test)]
mod tests;
