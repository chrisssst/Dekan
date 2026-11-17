use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::ltf::client::{BridgeClient, extract_patcher_state};
use crate::ltf::session::ops::{restore_disabled_mods, wait_patcher_stopped};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct LtfJournal {
    pub temporary_mod_ids: Vec<String>,
    pub disabled_user_mod_ids: Vec<String>,
}

#[must_use]
pub(super) fn default_journal_path() -> PathBuf {
    dekan_platform::paths::data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("Dekan"))
        .join("ltf_journal.json")
}

pub(super) fn save_journal(path: &Path, journal: &LtfJournal) {
    if journal.temporary_mod_ids.is_empty() && journal.disabled_user_mod_ids.is_empty() {
        clear_journal(path);
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent); // ignore-ok: best-effort dir create
    }
    let Ok(bytes) = serde_json::to_vec_pretty(journal) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, path).is_ok() {
        return;
    }
    let _ = std::fs::write(path, bytes); // ignore-ok: fallback direct journal write
}

pub(super) fn load_journal(path: &Path) -> LtfJournal {
    let Ok(bytes) = std::fs::read(path) else {
        return LtfJournal::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub(super) fn clear_journal(path: &Path) {
    let _ = std::fs::remove_file(path); // ignore-ok: best-effort journal removal
}

pub(super) async fn release_session(
    client: &BridgeClient,
    journal_path: &Path,
    imported_mods: &[String],
    disabled_user_mods: &[String],
) {
    let mut is_running = false;
    if let Ok(st) = client.patcher_status().await {
        (is_running, _, _) = extract_patcher_state(&st);
    }
    if is_running && !wait_patcher_stopped(client, Duration::from_secs(15)).await {
        warn!("Mod Manager patcher did not report idle within 15s on shutdown");
    }
    for mod_id in imported_mods {
        let _ = client.toggle_mod(mod_id, false).await; // ignore-ok: best-effort toggle
        let _ = client.uninstall_mod(mod_id).await; // ignore-ok: best-effort uninstall
    }
    restore_disabled_mods(client, disabled_user_mods.to_vec()).await;
    clear_journal(journal_path);
}

pub(super) struct SessionCleanupGuard {
    pub client: BridgeClient,
    pub journal_path: PathBuf,
    pub imported: Vec<String>,
    pub disabled_others: Vec<String>,
    pub active: bool,
}

impl SessionCleanupGuard {
    pub fn persist(&self) {
        save_journal(
            &self.journal_path,
            &LtfJournal {
                temporary_mod_ids: self.imported.clone(),
                disabled_user_mod_ids: self.disabled_others.clone(),
            },
        );
    }

    pub fn disarm(mut self) -> (Vec<String>, Vec<String>) {
        self.active = false;
        (
            std::mem::take(&mut self.imported),
            std::mem::take(&mut self.disabled_others),
        )
    }
}

impl Drop for SessionCleanupGuard {
    fn drop(&mut self) {
        if !self.active || (self.imported.is_empty() && self.disabled_others.is_empty()) {
            return;
        }
        let client = self.client.clone();
        let journal_path = self.journal_path.clone();
        let imported = std::mem::take(&mut self.imported);
        let disabled = std::mem::take(&mut self.disabled_others);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _lock = super::LTF_SESSION_LOCK.lock().await;
                release_session(&client, &journal_path, &imported, &disabled).await;
                let mut active = super::ACTIVE_DEKAN_MODS.lock().await;
                active.retain(|id| !imported.contains(id));
            });
        }
    }
}

pub(super) async fn recover_leftovers(client: &BridgeClient, path: &Path) {
    let journal = load_journal(path);
    if journal.temporary_mod_ids.is_empty() && journal.disabled_user_mod_ids.is_empty() {
        return;
    }
    warn!(
        temp_mods = journal.temporary_mod_ids.len(),
        disabled_mods = journal.disabled_user_mod_ids.len(),
        "Recovering leftover Mod Manager session state from an earlier run"
    );
    if let Ok(st) = client.patcher_status().await {
        let (running, _, _) = extract_patcher_state(&st);
        if running {
            let _ = wait_patcher_stopped(client, Duration::from_secs(10)).await; // ignore-ok: best-effort idle wait before leftover recovery
        }
    }
    for mod_id in &journal.temporary_mod_ids {
        let _ = client.toggle_mod(mod_id, false).await; // ignore-ok: best-effort untoggle
        let _ = client.uninstall_mod(mod_id).await; // ignore-ok: best-effort uninstall
    }
    restore_disabled_mods(client, journal.disabled_user_mod_ids).await;
    clear_journal(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_round_trips_and_clears_when_empty() {
        let dir = std::env::temp_dir().join(format!("dekan_ltf_journal_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir); // ignore-ok: temp dir
        let path = dir.join("ltf_journal.json");

        let sample = LtfJournal {
            temporary_mod_ids: vec!["temp-1".into()],
            disabled_user_mod_ids: vec!["user-1".into(), "user-2".into()],
        };
        save_journal(&path, &sample);
        assert_eq!(load_journal(&path), sample);

        save_journal(&path, &LtfJournal::default());
        assert!(!path.exists());

        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup temp dir
    }
}
