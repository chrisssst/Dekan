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
    version: &str,
    file: &str,
) -> Result<Vec<u8>, String> {
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

pub(super) async fn inspect(client: &reqwest::Client, version: &str) -> Result<Injector, String> {
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
            debug!(error = %e, "LTK Manager releases could not be checked; using the last known result");
            verdicts.status_from_cache().and_then(|s| s.compatible)
        }
    }
}
