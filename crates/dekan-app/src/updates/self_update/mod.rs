use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ring::signature::{ED25519, UnparsedPublicKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use dekan_core::phase::GamePhase;

use crate::update_check::{REPOSITORY, Version};

const SUMS_FILE: &str = "SHA256SUMS";
pub const SIGNATURE_FILE: &str = "SHA256SUMS.sig";
const FILE_SHARE_READ: u32 = 1;

pub const SILENT_INSTALL: [&str; 6] = [
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/CLOSEAPPLICATIONS",
    "/FORCECLOSEAPPLICATIONS",
    "/RELAUNCH=1",
];

const RELEASE_PUBLIC_KEY: [u8; 32] = [
    0x0a, 0x28, 0x5e, 0x29, 0x68, 0x63, 0xba, 0x57, 0x1c, 0x5b, 0xa2, 0xa0, 0x20, 0x96, 0x96, 0x5e,
    0xd6, 0x84, 0x4a, 0x82, 0xe1, 0xdd, 0x45, 0x13, 0xa1, 0xe1, 0xc9, 0x4a, 0xfb, 0x1a, 0x47, 0x3d,
];

const STAGING_DIR: &str = "update";
const UNINSTALLER: &str = "unins000.exe";
const USER_AGENT: &str = concat!("Dekan/", env!("CARGO_PKG_VERSION"), " (self update)");
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_SUMS_BYTES: usize = 16 * 1024;
const MAX_SETUP_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: Version,
    pub assets: Vec<ReleaseAsset>,
}

#[must_use]
fn setup_name(version: Version) -> String {
    format!("Dekan-Setup-{version}-x64.exe")
}

#[must_use]
pub fn is_installed_copy(exe: &Path) -> bool {
    exe.parent()
        .is_some_and(|dir| dir.join(UNINSTALLER).is_file())
}

pub fn verify_sums(sums: &[u8], signature: &[u8]) -> Result<(), String> {
    verify_sums_with(&RELEASE_PUBLIC_KEY, sums, signature)
}

fn verify_sums_with(public_key: &[u8], sums: &[u8], signature: &[u8]) -> Result<(), String> {
    let text =
        std::str::from_utf8(signature).map_err(|_| format!("{SIGNATURE_FILE} is not text"))?;
    let signature = decode_hex(text.trim())
        .ok_or_else(|| format!("{SIGNATURE_FILE} is not a hex signature"))?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(sums, &signature)
        .map_err(|_| format!("{SIGNATURE_FILE} does not match the release key"))
}

#[must_use]
fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        let valid = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        (valid && file == name).then(|| hash.to_ascii_lowercase())
    })
}

#[must_use]
pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[must_use]
pub fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

fn asset_url<'a>(offer: &'a Offer, name: &str) -> Result<&'a str, String> {
    let prefix = format!("{}/releases/download/", REPOSITORY.trim_end_matches('/'));
    let asset = offer
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| format!("release {} has no {name}", offer.version))?;
    if !asset.browser_download_url.starts_with(&prefix) {
        return Err(format!(
            "{name} is not served from the project releases: {}",
            asset.browser_download_url
        ));
    }
    Ok(&asset.browser_download_url)
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateError {
    Untrusted(String),
    Failed(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Untrusted(why) | Self::Failed(why) => f.write_str(why),
        }
    }
}

impl From<String> for UpdateError {
    fn from(why: String) -> Self {
        Self::Failed(why)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedSetup {
    pub path: PathBuf,
    pub sha256: String,
}

#[must_use]
pub fn can_install(phase: GamePhase) -> bool {
    matches!(
        phase,
        GamePhase::None | GamePhase::WaitingForStats | GamePhase::EndOfGame
    )
}

pub fn open_verified(setup: &StagedSetup) -> Result<std::fs::File, String> {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;

    let path = &setup.path;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let hash = encode_hex(&hasher.finalize());
    if hash != setup.sha256 {
        return Err(format!(
            "{} changed after it was verified: SHA-256 {hash}, signed {}",
            path.display(),
            setup.sha256
        ));
    }
    Ok(file)
}

#[derive(Debug, Clone, Default)]
pub struct PendingUpdate(Arc<Mutex<Option<StagedSetup>>>);

impl PendingUpdate {
    pub fn set(&self, setup: StagedSetup) -> Result<(), String> {
        let mut slot = self
            .0
            .lock()
            .map_err(|e| format!("the pending update lock is poisoned: {e}"))?;
        *slot = Some(setup);
        Ok(())
    }

    #[must_use]
    pub fn take(&self) -> Option<StagedSetup> {
        match self.0.lock() {
            Ok(mut slot) => slot.take(),
            Err(e) => {
                tracing::error!(error = %e, "The pending update lock is poisoned; the verified installer is not run");
                None
            }
        }
    }
}

pub fn clear_staging(state_dir: &Path) {
    match std::fs::remove_dir_all(state_dir.join(STAGING_DIR)) {
        Ok(()) => tracing::debug!("Previous update download removed"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            tracing::warn!(dir = %state_dir.join(STAGING_DIR).display(), error = %e, "Previous update download not removed")
        }
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|e| format!("the HTTP client could not be created: {e}"))
}

async fn open(client: &reqwest::Client, url: &str) -> Result<reqwest::Response, String> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    Ok(response)
}

async fn fetch_small(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let body = open(client, url)
        .await?
        .bytes()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if body.len() > MAX_SUMS_BYTES {
        return Err(format!("{url} is larger than {MAX_SUMS_BYTES} bytes"));
    }
    Ok(body.to_vec())
}

async fn fetch_to_file(client: &reqwest::Client, url: &str, path: &Path) -> Result<String, String> {
    let mut response = open(client, url).await?;
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|e| format!("{url}: {e}"))? {
        total += chunk.len() as u64;
        if total > MAX_SETUP_BYTES {
            return Err(format!("{url} is larger than {MAX_SETUP_BYTES} bytes"));
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    file.flush()
        .await
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(encode_hex(&hasher.finalize()))
}

pub async fn download(offer: &Offer, state_dir: &Path) -> Result<StagedSetup, UpdateError> {
    let client = client()?;
    let sums = fetch_small(&client, asset_url(offer, SUMS_FILE)?).await?;
    let signature = fetch_small(&client, asset_url(offer, SIGNATURE_FILE)?).await?;
    verify_sums(&sums, &signature).map_err(UpdateError::Untrusted)?;
    let name = setup_name(offer.version);
    let sums = String::from_utf8_lossy(&sums);
    let expected = expected_hash(&sums, &name)
        .ok_or_else(|| UpdateError::Untrusted(format!("{SUMS_FILE} does not list {name}")))?;
    let url = asset_url(offer, &name)?;

    clear_staging(state_dir);
    let dir = state_dir.join(STAGING_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let partial = dir.join(format!("{name}.partial"));
    let fetched = fetch_to_file(&client, url, &partial).await;
    let setup = dir.join(&name);
    let placed = match fetched {
        Ok(hash) if hash != expected => Err(UpdateError::Untrusted(format!(
            "{name} has SHA-256 {hash}, the signed list says {expected}"
        ))),
        Ok(_) => std::fs::rename(&partial, &setup)
            .map_err(|e| UpdateError::Failed(format!("{}: {e}", setup.display()))),
        Err(e) => Err(UpdateError::Failed(e)),
    };
    if let Err(e) = placed {
        clear_staging(state_dir);
        return Err(e);
    }
    Ok(StagedSetup {
        path: setup,
        sha256: expected,
    })
}

#[cfg(test)]
mod tests;
