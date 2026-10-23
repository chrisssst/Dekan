use super::*;

pub(super) fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

pub(super) async fn get(
    client: &reqwest::Client,
    url: &str,
    accept: &str,
) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, accept)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let rate_left = response
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");
        return Err(format!(
            "GitHub answered HTTP {} for {url} (rate limit remaining: {rate_left})",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_BINARY_BYTES as u64)
    {
        return Err(format!("{url} is larger than {MAX_BINARY_BYTES} bytes"));
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > MAX_BINARY_BYTES {
            return Err(format!("{url} is larger than {MAX_BINARY_BYTES} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[must_use]
pub fn is_release_version(version: &str) -> bool {
    (1..=32).contains(&version.len())
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

pub(super) async fn release_commit(
    client: &reqwest::Client,
    version: &str,
) -> Result<String, String> {
    if !is_release_version(version) {
        return Err(format!("{version:?} is not an LTK Manager release version"));
    }
    let body = get(
        client,
        &format!("{LTK_API}/commits/v{version}"),
        "application/vnd.github.sha",
    )
    .await?;
    let commit = String::from_utf8_lossy(&body).trim().to_owned();
    if commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(commit)
    } else {
        Err(format!("release {version} did not resolve to a commit"))
    }
}

pub(super) async fn release_versions(client: &reqwest::Client) -> Result<Vec<String>, String> {
    let body = get(
        client,
        &format!("{LTK_API}/releases?per_page={RELEASES_PER_PAGE}"),
        "application/vnd.github+json",
    )
    .await?;
    published_versions(&String::from_utf8_lossy(&body))
}

pub(super) async fn resource(
    client: &reqwest::Client,
    commit: &str,
    file: &str,
) -> Result<Vec<u8>, String> {
    let url = format!("{LTK_API}/contents/{RESOURCES_PATH}/{file}?ref={commit}");
    get(client, &url, "application/vnd.github.raw").await
}

pub async fn download_injector(version: &str) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let client = http_client()?;
    let commit = release_commit(&client, version).await?;
    let mut files = Vec::with_capacity(INJECTOR_FILES.len());
    for name in INJECTOR_FILES {
        files.push((name, resource(&client, &commit, name).await?));
    }
    info!(version, commit = %commit, "Injector downloaded from the commit the LTK Manager release tag points at");
    Ok(files)
}

pub(super) async fn inspect(client: &reqwest::Client, version: &str) -> Result<Injector, String> {
    let dir = std::env::temp_dir().join(format!(
        "dekan_ltk_inspect_{}_{version}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let inspected = inspect_files(client, version, &dir).await;
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        debug!(error = %e, "LTK inspection folder not removed");
    }
    inspected
}

async fn inspect_files(
    client: &reqwest::Client,
    version: &str,
    dir: &Path,
) -> Result<Injector, String> {
    let commit = match release_commit(client, version).await {
        Ok(commit) => commit,
        Err(e) if e.contains(NOT_FOUND_MARK) => {
            warn!(version, error = %e, "An LTK Manager release has no tag to read the injector from; it is treated as untrusted");
            return Ok(Injector::Untrusted);
        }
        Err(e) => return Err(format!("release {version}: {e}")),
    };
    let mut dll_sha256 = String::new();
    let mut trusted = true;
    for name in INJECTOR_FILES {
        let bytes = match resource(client, &commit, name).await {
            Ok(bytes) => bytes,
            Err(e) if e.contains(NOT_FOUND_MARK) => {
                warn!(version, file = name, error = %e, "An LTK Manager release lacks an injector file; it is treated as untrusted");
                return Ok(Injector::Untrusted);
            }
            Err(e) => return Err(format!("release {version}, {name}: {e}")),
        };
        let path = dir.join(name);
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        match dekan_inject::trust::verify_injector_file(&path) {
            Ok(dekan_inject::trust::Provenance::Signed) => {}
            Ok(dekan_inject::trust::Provenance::Unsigned) => {
                info!(
                    version,
                    file = name,
                    "An LTK Manager release ships this injector file without a signature; it is accepted as published by the official repository"
                );
            }
            Err(e) => {
                warn!(version, file = name, reason = %e, "An LTK Manager release carries an injector with a foreign or broken signature; Dekan does not use it");
                trusted = false;
            }
        }
        if name == INJECTOR_FILES[1] {
            if dekan_inject::trust::dll_refuses_other_hosts(&bytes) {
                warn!(
                    version,
                    "An LTK Manager release ships a patcher DLL that refuses any host but LTK Manager; Dekan does not use it"
                );
                return Ok(Injector::Untrusted);
            }
            dll_sha256 = dekan_inject::dll_validator::compute_sha256(&bytes);
        }
    }
    Ok(if trusted {
        Injector::Trusted { dll_sha256 }
    } else {
        Injector::Untrusted
    })
}

pub(super) async fn refresh(
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
            warn!(error = %e, "LTK Manager releases could not be checked; using the last known result");
            verdicts.status_from_cache().and_then(|s| s.compatible)
        }
    }
}
