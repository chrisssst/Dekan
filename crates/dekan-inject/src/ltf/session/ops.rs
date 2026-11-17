use std::time::Duration;

use tracing::{info, warn};

use crate::error::InjectError;
use crate::ltf::client::{BridgeClient, extract_patcher_state};

pub(super) async fn wait_patcher_stopped(client: &BridgeClient, timeout: Duration) -> bool {
    let _ = client.stop_patcher().await; // ignore-ok: request active patcher to stop
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(st) = client.patcher_status().await {
            let (running, _, _) = extract_patcher_state(&st);
            if !running {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

pub(super) async fn start_patcher_with_retry(client: &BridgeClient) -> Result<(), InjectError> {
    match client.start_patcher().await {
        Ok(()) => Ok(()),
        Err(first_err) => {
            let msg = first_err.to_string().to_ascii_lowercase();
            if !msg.contains("alreadyrunning") && !msg.contains("already running") {
                return Err(first_err);
            }
            warn!("Mod Manager patcher reported AlreadyRunning; waiting for prior run to finish");
            if !wait_patcher_stopped(client, Duration::from_secs(15)).await {
                warn!("Mod Manager patcher did not stop within 15s before retry");
            }
            client.start_patcher().await
        }
    }
}

pub(super) async fn disable_other_enabled_mods(
    client: &BridgeClient,
    keep_ids: &[String],
) -> Vec<String> {
    let mods = match client.list_mods().await {
        Ok(mods) => mods,
        Err(e) => {
            warn!(error = %e, "Could not list Mod Manager library to isolate enabled mods");
            return Vec::new();
        }
    };
    let mut disabled = Vec::new();
    for entry in mods {
        if entry.enabled != Some(true) || keep_ids.contains(&entry.id) {
            continue;
        }
        match client.toggle_mod(&entry.id, false).await {
            Ok(()) => {
                info!(mod_id = %entry.id, name = ?entry.name, "Temporarily disabled non-session Mod Manager mod");
                disabled.push(entry.id);
            }
            Err(e) => {
                warn!(mod_id = %entry.id, error = %e, "Could not disable non-session Mod Manager mod");
            }
        }
    }
    disabled
}

pub(super) async fn restore_disabled_mods(client: &BridgeClient, disabled: Vec<String>) {
    for mod_id in disabled {
        let _ = client.toggle_mod(&mod_id, true).await; // ignore-ok: best-effort restore mod
    }
}
