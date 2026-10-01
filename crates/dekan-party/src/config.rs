use std::path::Path;

use crate::error::PartyError;

pub const RELAY_ENV: &str = dekan_core::env::RELAY_URL;

pub const RELAY_FILE: &str = "party.json";

pub const DEFAULT_RELAY_URL: &str = "wss://bullet-party-relay.longhaired-angora.workers.dev";

#[derive(serde::Deserialize)]
struct PartyFile {
    relay_url: String,
}

pub fn relay_url(state_dir: &Path) -> Result<String, PartyError> {
    let raw = match std::env::var(RELAY_ENV) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            let path = state_dir.join(RELAY_FILE);
            if !path.exists() {
                // ignore-ok: best-effort creation; if it fails (e.g. read-only filesystem), fallback to DEFAULT_RELAY_URL
                let _ = std::fs::create_dir_all(state_dir);
                let default_content = format!("{{\n  \"relay_url\": \"{DEFAULT_RELAY_URL}\"\n}}\n");

                // ignore-ok: best-effort write; if it fails, fallback to DEFAULT_RELAY_URL
                let _ = std::fs::write(&path, default_content);
                DEFAULT_RELAY_URL.to_string()
            } else {
                let bytes = std::fs::read(&path).map_err(|e| {
                    PartyError::NoRelay(format!("failed to read {}: {e}", path.display()))
                })?;
                let file: PartyFile = serde_json::from_slice(&bytes).map_err(|e| {
                    PartyError::NoRelay(format!("{} is not valid: {e}", path.display()))
                })?;
                file.relay_url
            }
        }
    };
    validate_relay_url(raw.trim())
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
}
