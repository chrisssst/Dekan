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

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

async fn get(client: &reqwest::Client, url: &str, accept: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, accept)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("GitHub answered HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_BINARY_BYTES as u64)
    {
        return Err(format!("{url} is larger than {MAX_BINARY_BYTES} bytes"));
    }
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    if body.len() > MAX_BINARY_BYTES {
        return Err(format!("{url} is larger than {MAX_BINARY_BYTES} bytes"));
    }
    Ok(body.to_vec())
}

async fn release_versions(client: &reqwest::Client) -> Result<Vec<String>, String> {
    let body = get(
        client,
        &format!("{LTK_API}/releases?per_page={RELEASES_PER_PAGE}"),
        "application/vnd.github+json",
    )
    .await?;
    published_versions(&String::from_utf8_lossy(&body))
}

async fn resource(client: &reqwest::Client, version: &str, file: &str) -> Result<Vec<u8>, String> {
    let url = format!("{LTK_API}/contents/{RESOURCES_PATH}/{file}?ref=v{version}");
    get(client, &url, "application/vnd.github.raw").await
}

pub async fn download_injector(version: &str) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let client = http_client()?;
    let mut files = Vec::with_capacity(INJECTOR_FILES.len());
    for name in INJECTOR_FILES {
        files.push((name, resource(&client, version, name).await?));
    }
    Ok(files)
}

async fn inspect(client: &reqwest::Client, version: &str) -> Result<Injector, String> {
    let dir = std::env::temp_dir().join(format!(
        "dekan_ltk_inspect_{}_{version}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut dll_sha256 = String::new();
    let mut trusted = true;
    for name in INJECTOR_FILES {
        let bytes = resource(client, version, name).await?;
        let path = dir.join(name);
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        if let Err(e) = dekan_inject::trust::verify_injector_file(&path) {
            info!(version, file = name, reason = %e, "An LTK Manager release carries an injector Dekan does not trust");
            trusted = false;
        }
        if name == INJECTOR_FILES[1] {
            dll_sha256 = dekan_inject::dll_validator::compute_sha256(&bytes);
        }
    }
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        debug!(error = %e, "LTK inspection folder not removed");
    }
    Ok(if trusted {
        Injector::Trusted { dll_sha256 }
    } else {
        Injector::Untrusted
    })
}

async fn refresh(
    client: &reqwest::Client,
    verdicts: &mut Verdicts,
) -> Result<Option<LtkStatus>, String> {
    let versions = release_versions(client).await?;
    let mut inspected = 0;
    for version in &versions {
        let trusted = match verdicts.get(version) {
            Some(known) => matches!(known, Injector::Trusted { .. }),
            None if inspected < MAX_INSPECTIONS_PER_CHECK => {
                inspected += 1;
                let injector = inspect(client, version).await?;
                let trusted = matches!(injector, Injector::Trusted { .. });
                verdicts.insert(version, injector);
                trusted
            }
            None => break,
        };
        if trusted {
            break;
        }
    }
    Ok(verdicts.status(&versions))
}

pub async fn compatible_version(state_dir: &Path) -> Option<String> {
    let mut verdicts = load_verdicts(state_dir);
    let refreshed = match http_client() {
        Ok(client) => refresh(&client, &mut verdicts).await,
        Err(e) => Err(e),
    };
    save_verdicts(state_dir, &verdicts);
    match refreshed {
        Ok(status) => status.and_then(|s| s.compatible),
        Err(e) => {
            debug!(error = %e, "LTK Manager releases could not be checked; using the last known result");
            verdicts.status_from_cache().and_then(|s| s.compatible)
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const DLL_A: &str = "6d419057e6667994ba752ad0fb089b363db98267618644d7f7b6632441a21d74";
    const DLL_B: &str = "07a43bf36a389eb00f6276e333bd7f2b95218f25a58e1e128ff4d2e4ab2dc99b";

    fn list(versions: &[&str]) -> Vec<String> {
        versions.iter().map(|v| (*v).to_owned()).collect()
    }

    fn trusted(dll: &str) -> Injector {
        Injector::Trusted {
            dll_sha256: dll.to_owned(),
        }
    }

    #[test]
    fn published_versions_come_newest_first_without_drafts_or_prereleases() {
        let body = r#"[
            {"tag_name":"v1.24.0"},
            {"tag_name":"v1.26.1","draft":false,"prerelease":false},
            {"tag_name":"v1.27.0","prerelease":true},
            {"tag_name":"v1.28.0","draft":true},
            {"tag_name":"nightly"},
            {"tag_name":"v1.26.0"}
        ]"#;
        assert_eq!(
            published_versions(body),
            Ok(list(&["1.26.1", "1.26.0", "1.24.0"]))
        );
        assert!(published_versions("not json").is_err());
    }

    #[test]
    fn the_compatible_version_is_the_newest_release_signed_by_the_publisher() {
        let mut verdicts = Verdicts::default();
        verdicts.insert("1.28.0", Injector::Untrusted);
        verdicts.insert("1.27.0", trusted(DLL_A));
        verdicts.insert("1.26.1", trusted(DLL_B));
        let status = verdicts
            .status(&list(&["1.28.0", "1.27.0", "1.26.1"]))
            .expect("status");
        assert_eq!(status.latest, "1.28.0");
        assert!(!status.latest_trusted);
        assert_eq!(status.compatible.as_deref(), Some("1.27.0"));
        assert_eq!(status.compatible_dll.as_deref(), Some(DLL_A));
        assert_eq!(verdicts.status(&list(&["9.9.9"])), None);
        assert_eq!(verdicts.status(&[]), None);
    }

    #[test]
    fn an_update_is_offered_only_when_the_installed_dll_differs() {
        let status = LtkStatus {
            latest: "1.27.0".into(),
            latest_trusted: true,
            compatible: Some("1.27.0".into()),
            compatible_dll: Some(DLL_A.into()),
        };
        assert_eq!(status.offers_update_over(Some(DLL_B)), Some("1.27.0"));
        assert_eq!(
            status.offers_update_over(None),
            Some("1.27.0"),
            "missing files"
        );
        assert_eq!(status.offers_update_over(Some(DLL_A)), None);
        assert_eq!(
            status.offers_update_over(Some(&DLL_A.to_ascii_uppercase())),
            None
        );
        let nothing_trusted = LtkStatus {
            compatible: None,
            compatible_dll: None,
            ..status
        };
        assert_eq!(nothing_trusted.offers_update_over(Some(DLL_B)), None);
    }

    #[test]
    fn the_cache_orders_versions_numerically() {
        let mut verdicts = Verdicts::default();
        verdicts.insert("1.9.0", trusted(DLL_B));
        verdicts.insert("1.10.0", Injector::Untrusted);
        let status = verdicts.status_from_cache().expect("status");
        assert_eq!(status.latest, "1.10.0");
        assert_eq!(status.compatible.as_deref(), Some("1.9.0"));
    }

    #[test]
    fn verdicts_round_trip_and_drop_what_they_cannot_read() {
        let dir = std::env::temp_dir().join(format!("dekan_ltk_check_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&dir).expect("fixture dir");
        assert_eq!(load_verdicts(&dir), Verdicts::default());

        let mut verdicts = Verdicts::default();
        verdicts.insert("1.27.0", trusted(DLL_A));
        verdicts.insert("1.28.0", Injector::Untrusted);
        save_verdicts(&dir, &verdicts);
        assert_eq!(load_verdicts(&dir), verdicts);

        std::fs::write(dir.join(VERDICTS_FILE), "old-format-line\n1.27.0 audited\n")
            .expect("write");
        assert_eq!(
            load_verdicts(&dir),
            Verdicts::default(),
            "a cache from the hash era is dropped"
        );

        let garbage = format!(
            "{}\n1.26.1 maybe\nnot-a-version untrusted\n1.27.0 trusted short\n1.27.1 trusted {DLL_A} extra\n",
            fingerprint()
        );
        std::fs::write(dir.join(VERDICTS_FILE), garbage).expect("write");
        assert_eq!(load_verdicts(&dir), Verdicts::default());
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
    }

    #[test]
    fn a_new_injector_is_announced_once() {
        let dir = std::env::temp_dir().join(format!("dekan_ltk_notice_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&dir).expect("fixture dir");
        assert!(!already_notified(&dir, "1.27.0"));
        remember_notified(&dir, "1.27.0");
        assert!(already_notified(&dir, "1.27.0"));
        assert!(!already_notified(&dir, "1.28.0"));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
    }

    #[tokio::test]
    #[ignore = "downloads LTK Manager releases from GitHub"]
    async fn the_newest_signed_release_is_found_online() {
        let dir = std::env::temp_dir().join(format!("dekan_ltk_online_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&dir).expect("fixture dir");
        let version = compatible_version(&dir).await.expect("a signed release");
        let verdicts = load_verdicts(&dir);
        assert!(matches!(
            verdicts.get(&version),
            Some(Injector::Trusted { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
    }

    #[test]
    fn pages_point_at_the_ltk_manager_releases() {
        assert_eq!(
            release_page(Some("1.27.0")),
            format!("{LTK_REPOSITORY}/releases/tag/v1.27.0")
        );
        assert_eq!(release_page(None), format!("{LTK_REPOSITORY}/releases"));
    }
}
