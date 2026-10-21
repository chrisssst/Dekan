use std::path::Path;

use crate::error::PartyError;

pub const RELAY_ENV: &str = dekan_core::env::RELAY_URL;

pub const RELAY_FILE: &str = "party.json";

pub const DEFAULT_RELAY_URL: &str = "wss://dekan-party-relay.heeazk.workers.dev";

const LEGACY_DEFAULT_RELAY_URL: &str = "wss://bullet-party-relay.longhaired-angora.workers.dev";

#[derive(serde::Deserialize, serde::Serialize)]
struct PartyFile {
    relay_url: String,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

pub fn relay_url(state_dir: &Path) -> Result<String, PartyError> {
    let from_env = match std::env::var(RELAY_ENV) {
        Ok(url) if !url.trim().is_empty() => Some(url),
        Err(std::env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                variable = RELAY_ENV,
                "The relay variable is not valid Unicode; using party.json"
            );
            None
        }
        _ => None,
    };
    let raw = match from_env {
        Some(url) => url,
        None => {
            let path = state_dir.join(RELAY_FILE);
            if !path.exists() {
                let default_content = format!(
                    "{{
  \"relay_url\": \"{DEFAULT_RELAY_URL}\"
}}
"
                );
                let written = std::fs::create_dir_all(state_dir)
                    .and_then(|()| std::fs::write(&path, default_content));
                if let Err(e) = written {
                    tracing::warn!(path = %path.display(), error = %e, "Could not write the default party.json; using the built-in relay");
                }
                DEFAULT_RELAY_URL.to_string()
            } else {
                read_relay_file(&path)?
            }
        }
    };
    validate_relay_url(raw.trim())
}

fn read_relay_file(path: &Path) -> Result<String, PartyError> {
    let bytes = std::fs::read(path)
        .map_err(|e| PartyError::NoRelay(format!("failed to read {}: {e}", path.display())))?;
    let mut file: PartyFile = serde_json::from_slice(&bytes)
        .map_err(|e| PartyError::NoRelay(format!("{} is not valid: {e}", path.display())))?;
    if file.relay_url.trim().trim_end_matches('/') == LEGACY_DEFAULT_RELAY_URL {
        file.relay_url = DEFAULT_RELAY_URL.to_owned();
        let serialized = serde_json::to_vec_pretty(&file)
            .map_err(|e| PartyError::NoRelay(format!("relay preference: {e}")))?;
        let temporary = path.with_extension("json.new");
        let written =
            std::fs::write(&temporary, serialized).and_then(|()| std::fs::rename(&temporary, path));
        if let Err(e) = written {
            tracing::warn!(path = %path.display(), error = %e, "Could not save the updated relay preference; using the Dekan relay for this session");
        }
    }
    Ok(file.relay_url)
}

pub fn validate_relay_url(url: &str) -> Result<String, PartyError> {
    let url = url.trim_end_matches('/');
    let host_part = if let Some(rest) = url.strip_prefix("wss://") {
        rest
    } else if let Some(rest) = url.strip_prefix("ws://") {
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        if host != "127.0.0.1" && host != "localhost" {
            return Err(PartyError::NoRelay(
                "ws:// is only accepted for 127.0.0.1/localhost; use wss://".into(),
            ));
        }
        rest
    } else {
        return Err(PartyError::NoRelay(
            "the relay URL must start with wss://".into(),
        ));
    };
    if host_part.is_empty() || url.contains(['?', '#', ' ']) {
        return Err(PartyError::NoRelay("the relay URL is malformed".into()));
    }
    Ok(url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_only_secure_or_loopback_urls_are_accepted() {
        assert_eq!(
            validate_relay_url("wss://relay.example.workers.dev/").expect("ok"),
            "wss://relay.example.workers.dev"
        );
        assert!(validate_relay_url("ws://127.0.0.1:8787").is_ok());
        assert!(validate_relay_url("ws://relay.example.com").is_err());
        assert!(validate_relay_url("https://relay.example.com").is_err());
        assert!(validate_relay_url("wss://").is_err());
        assert!(validate_relay_url("wss://x.dev/room?key=1").is_err());
    }

    #[test]
    fn test_auto_creates_party_json_if_missing() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = std::env::temp_dir().join(format!("dekan_party_test_{unique}"));
        let url = relay_url(&tmp).expect("auto create party.json");
        assert_eq!(url, DEFAULT_RELAY_URL);
        assert!(tmp.join(RELAY_FILE).exists());

        // ignore-ok: test cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }
    fn fixture(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dekan_relay_config_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("fixture directory");
        dir.join(RELAY_FILE)
    }

    #[test]
    fn saved_legacy_default_migrates_and_preserves_other_settings() {
        let path = fixture("legacy");
        for suffix in ["", "/", "/  "] {
            std::fs::write(
                &path,
                serde_json::to_vec(&serde_json::json!({
                    "relay_url": format!("  {LEGACY_DEFAULT_RELAY_URL}{suffix}"),
                    "other_setting": {"keep": true}
                }))
                .expect("serialize fixture"),
            )
            .expect("write fixture");
            assert_eq!(
                read_relay_file(&path).expect("migrate default"),
                DEFAULT_RELAY_URL
            );
            let saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).expect("read migrated file"))
                    .expect("valid JSON");
            assert_eq!(saved["relay_url"], DEFAULT_RELAY_URL);
            assert_eq!(saved["other_setting"]["keep"], true);
            assert_eq!(
                read_relay_file(&path).expect("read again"),
                DEFAULT_RELAY_URL
            );
        }
        let _ = std::fs::remove_dir_all(path.parent().expect("fixture parent")); // ignore-ok: fixture cleanup
    }

    #[test]
    fn saved_custom_relay_and_malformed_file_are_not_rewritten() {
        let path = fixture("custom");
        let custom = br#"{ "relay_url": "wss://custom.example.com", "keep": 42 }"#;
        std::fs::write(&path, custom).expect("write custom relay");
        assert_eq!(
            read_relay_file(&path).expect("custom relay"),
            "wss://custom.example.com"
        );
        assert_eq!(std::fs::read(&path).expect("read custom"), custom);
        std::fs::write(&path, b"not JSON").expect("write invalid fixture");
        assert!(read_relay_file(&path).is_err());
        assert_eq!(std::fs::read(&path).expect("read invalid"), b"not JSON");
        let _ = std::fs::remove_dir_all(path.parent().expect("fixture parent")); // ignore-ok: fixture cleanup
    }
}
