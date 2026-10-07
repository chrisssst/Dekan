use std::io::{Cursor, Read};
use std::path::Path;
use std::time::Duration;

use dekan_platform::fs::{ExtractLimits, atomic_write, validate_archive_path};
use serde::Deserialize;
use thiserror::Error;
use tracing::{debug, info, warn};
use zip::ZipArchive;

const APP_USER_AGENT: &str = concat!(
    "Dekan/",
    env!("CARGO_PKG_VERSION"),
    " (League of Legends skin manager)"
);

pub const VERSION_FILE_NAME: &str = ".skin_version";

#[derive(Debug, Error)]
pub enum SkinSyncError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Zip extraction error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("Platform filesystem error: {0}")]
    Platform(#[from] dekan_platform::error::PlatformError),

    #[error("Invalid repository response: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkinSyncResult {
    UpToDate { sha: String },

    Updated { sha: String, files_extracted: usize },

    Skipped { reason: String },
}

#[derive(Debug, Deserialize)]
struct GithubCommitResponse {
    sha: String,
}

#[derive(Debug, Clone)]
pub struct SkinSyncConfig {
    pub api_base: String,
    pub zip_url: String,
    pub timeout: Duration,
}

impl SkinSyncConfig {
    #[must_use]
    pub fn from_env_value(value: Option<&str>) -> Option<Self> {
        let (owner, repo) = value?.trim().split_once('/')?;
        let valid = |part: &str| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        if !valid(owner) || !valid(repo) {
            return None;
        }
        Some(Self {
            api_base: format!("https://api.github.com/repos/{owner}/{repo}"),
            zip_url: format!("https://github.com/{owner}/{repo}/archive/refs/heads/main.zip"),
            timeout: Duration::from_secs(30),
        })
    }
}

pub fn dir_has_skins(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    std::fs::read_dir(dir)
        .map(|mut entries| {
            entries.any(|e| {
                e.ok().is_some_and(|entry| {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();

                    !name_str.starts_with('.')
                })
            })
        })
        .unwrap_or(false)
}

#[must_use]
pub fn get_local_sha(library_dir: &Path) -> Option<String> {
    let version_file = library_dir.join(VERSION_FILE_NAME);
    std::fs::read_to_string(version_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn save_local_sha(library_dir: &Path, sha: &str) -> Result<(), SkinSyncError> {
    let version_file = library_dir.join(VERSION_FILE_NAME);
    atomic_write(&version_file, sha.as_bytes(), false)?;
    Ok(())
}

pub async fn fetch_remote_sha(
    client: &reqwest::Client,
    api_base: &str,
) -> Result<String, SkinSyncError> {
    let url = format!("{api_base}/commits/main");
    let resp = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, APP_USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/vnd.github.v3+json")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(SkinSyncError::InvalidResponse(format!(
            "GitHub API returned HTTP {}",
            resp.status()
        )));
    }

    let commit: GithubCommitResponse = resp.json().await?;
    Ok(commit.sha)
}

pub async fn download_and_extract_skins(
    client: &reqwest::Client,
    zip_url: &str,
    library_dir: &Path,
    limits: &ExtractLimits,
) -> Result<usize, SkinSyncError> {
    info!(url = %zip_url, "Downloading skin repository archive...");

    let resp = client
        .get(zip_url)
        .header(reqwest::header::USER_AGENT, APP_USER_AGENT)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(SkinSyncError::InvalidResponse(format!(
            "ZIP download returned HTTP {}",
            resp.status()
        )));
    }

    if resp
        .content_length()
        .is_some_and(|len| len > limits.max_total_bytes)
    {
        return Err(SkinSyncError::InvalidResponse(format!(
            "the archive is larger than {} bytes",
            limits.max_total_bytes
        )));
    }
    let bytes = resp.bytes().await?;
    info!(
        bytes = bytes.len(),
        "Repository archive downloaded; beginning safe extraction"
    );

    let library_dir = library_dir.to_path_buf();
    let limits = limits.clone();
    tokio::task::spawn_blocking(move || extract_skins(bytes, &library_dir, &limits))
        .await
        .map_err(|e| SkinSyncError::Io(std::io::Error::other(e)))?
}

fn extract_skins<B: AsRef<[u8]>>(
    bytes: B,
    library_dir: &Path,
    limits: &ExtractLimits,
) -> Result<usize, SkinSyncError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;

    let mut extracted_count = 0usize;
    let mut total_bytes_extracted = 0u64;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let raw_name = entry.name().to_string();

        let relative_skin_path = if let Some(idx) = raw_name.find("/skins/") {
            &raw_name[idx + "/skins/".len()..]
        } else if let Some(stripped) = raw_name.strip_prefix("skins/") {
            stripped
        } else {
            continue;
        };

        if relative_skin_path.is_empty() || relative_skin_path.ends_with('/') {
            continue;
        }

        let validated_rel = validate_archive_path(Path::new(relative_skin_path))?;
        let target_path = library_dir.join(&validated_rel);

        let entry_size = entry.size();
        if entry_size > limits.max_single_file_bytes {
            warn!(
                entry = %raw_name,
                size = entry_size,
                max = limits.max_single_file_bytes,
                "Skipping archive entry exceeding single-file limit"
            );
            continue;
        }

        total_bytes_extracted += entry_size;
        if total_bytes_extracted > limits.max_total_bytes {
            warn!(
                total = total_bytes_extracted,
                max = limits.max_total_bytes,
                "Archive extraction stopped: total byte limit exceeded"
            );
            break;
        }

        let mut buf = Vec::with_capacity(entry_size as usize);
        (&mut entry).take(entry_size + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 != entry_size {
            return Err(SkinSyncError::InvalidResponse(format!(
                "{raw_name} unpacked to {} bytes, not the {entry_size} it declares",
                buf.len()
            )));
        }

        atomic_write(&target_path, &buf, false)?;
        extracted_count += 1;

        if extracted_count >= limits.max_entries {
            warn!(
                count = extracted_count,
                max = limits.max_entries,
                "Archive extraction stopped: entry count limit reached"
            );
            break;
        }
    }

    info!(
        files = extracted_count,
        target = %library_dir.display(),
        "Skin library extraction complete"
    );

    Ok(extracted_count)
}

pub async fn sync_skin_library(
    library_dir: &Path,
    config: &SkinSyncConfig,
    force: bool,
) -> Result<SkinSyncResult, SkinSyncError> {
    let client = reqwest::Client::builder().timeout(config.timeout).build()?;

    let has_skins = dir_has_skins(library_dir);
    let local_sha = get_local_sha(library_dir);

    debug!(
        library = %library_dir.display(),
        has_skins = has_skins,
        local_sha = ?local_sha,
        force = force,
        "Checking for skin library updates"
    );

    let remote_sha = match fetch_remote_sha(&client, &config.api_base).await {
        Ok(sha) => sha,
        Err(e) => {
            if has_skins {
                warn!(
                    error = %e,
                    "Could not check upstream skin repository; continuing with existing offline skins"
                );
                return Ok(SkinSyncResult::Skipped {
                    reason: format!("offline fallback: {e}"),
                });
            }

            return Err(e);
        }
    };

    if !force && has_skins && local_sha.as_deref() == Some(&remote_sha) {
        info!(
            sha = %remote_sha,
            "Skin library is up to date with upstream repository"
        );
        return Ok(SkinSyncResult::UpToDate { sha: remote_sha });
    }

    let limits = ExtractLimits::default();
    let extracted =
        download_and_extract_skins(&client, &config.zip_url, library_dir, &limits).await?;

    let _ = save_local_sha(library_dir, &remote_sha); // ignore-ok: failure to write version file does not invalidate the extracted files

    Ok(SkinSyncResult::Updated {
        sha: remote_sha,
        files_extracted: extracted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_is_off_unless_a_repository_is_named() {
        for off in [
            None,
            Some(""),
            Some("1"),
            Some("true"),
            Some("owner"),
            Some("/repo"),
        ] {
            assert!(SkinSyncConfig::from_env_value(off).is_none(), "{off:?}");
        }
        for bad in [
            "../repo",
            "owner/..",
            "own er/repo",
            "owner/re/po",
            "owner/repo?x=1",
        ] {
            assert!(SkinSyncConfig::from_env_value(Some(bad)).is_none(), "{bad}");
        }
        let config = SkinSyncConfig::from_env_value(Some(" someone/skin-library ")).expect("valid");
        assert_eq!(
            config.api_base,
            "https://api.github.com/repos/someone/skin-library"
        );
        assert_eq!(
            config.zip_url,
            "https://github.com/someone/skin-library/archive/refs/heads/main.zip"
        );
    }
    use std::io::Write;
    use std::path::PathBuf;
    use zip::write::SimpleFileOptions;

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dekan_test_skins_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: best effort test cleanup
        let _ = std::fs::create_dir_all(&dir); // ignore-ok: test setup
        dir
    }

    #[test]
    fn test_dir_has_skins_detects_numeric_folders() {
        let temp = temp_test_dir("numeric");
        assert!(!dir_has_skins(&temp));

        std::fs::write(temp.join(".skin_version"), "abc123").unwrap();
        assert!(!dir_has_skins(&temp));

        std::fs::create_dir(temp.join("238")).unwrap();
        assert!(dir_has_skins(&temp));

        let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
    }

    #[test]
    fn test_local_sha_roundtrip() {
        let temp = temp_test_dir("sha");
        assert_eq!(get_local_sha(&temp), None);

        save_local_sha(&temp, "deadbeef12345678").unwrap();
        assert_eq!(get_local_sha(&temp), Some("deadbeef12345678".to_string()));

        let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
    }

    #[test]
    fn test_safe_extraction_strips_repo_prefix() {
        let temp = temp_test_dir("extract");
        let zip_path = temp.join("test_skins.zip");

        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = SimpleFileOptions::default();

            zip.start_file("skin-library-main/skins/238/238001/238001.fantome", options)
                .unwrap();
            zip.write_all(b"mock_skin_content").unwrap();

            zip.finish().unwrap();
        }

        let zip_bytes = std::fs::read(&zip_path).unwrap();
        let cursor = Cursor::new(zip_bytes);
        let mut archive = ZipArchive::new(cursor).unwrap();
        let target_dir = temp.join("library");
        std::fs::create_dir_all(&target_dir).unwrap();

        let _limits = ExtractLimits::default();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            let raw_name = entry.name().to_string();
            if let Some(idx) = raw_name.find("/skins/") {
                let rel = &raw_name[idx + "/skins/".len()..];
                let validated = validate_archive_path(Path::new(rel)).unwrap();
                let dest = target_dir.join(&validated);
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf).unwrap();
                atomic_write(&dest, &buf, false).unwrap();
            }
        }

        let extracted_file = target_dir.join("238").join("238001").join("238001.fantome");
        assert!(extracted_file.is_file());
        assert_eq!(std::fs::read(extracted_file).unwrap(), b"mock_skin_content");

        let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
    }
}
