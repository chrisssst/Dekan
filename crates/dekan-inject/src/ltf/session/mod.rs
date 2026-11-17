use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dekan_core::state::StateSender;
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{info, warn};

use super::client::BridgeClient;
use super::launcher::ensure_bridge_running;
use super::log_tail::ManagerLogTail;
use crate::error::InjectError;
use crate::overlay_process::{OverlayProcess, PatcherSignal};

pub(super) static LTF_SESSION_LOCK: Mutex<()> = Mutex::const_new(());
pub(super) static ACTIVE_DEKAN_MODS: Mutex<Vec<String>> = Mutex::const_new(Vec::new());

pub async fn warmup_bridge(tools_dirs: &[PathBuf]) {
    let _lock = LTF_SESSION_LOCK.lock().await;
    if let Ok(client) = ensure_bridge_running(tools_dirs, Duration::from_secs(30)).await {
        if wait_for_cdp_ready(&client, Duration::from_secs(25))
            .await
            .is_ok()
        {
            let _ = client.health_check_off().await; // ignore-ok: warmup health check toggle
            let journal_path = journal::default_journal_path();
            journal::recover_leftovers(&client, &journal_path).await;
            info!(port = client.port(), "LTF bridge warmed up and ready");
        }
    }
}

pub async fn wait_for_cdp_ready(
    client: &BridgeClient,
    timeout: Duration,
) -> Result<(), InjectError> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(st) = client.status().await {
            let cdp_ok = st
                .get("cdp")
                .and_then(|c| c.get("available"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if cdp_ok && client.list_mods().await.is_ok() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(InjectError::Process(
        "Timed out waiting for Mod Manager frontend (CDP/Tauri) to become ready".into(),
    ))
}

pub fn pack_mod_fantome(mod_dir: &Path, staging_file: &Path) -> Result<(), InjectError> {
    if let Some(parent) = staging_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_file = staging_file.with_extension("fantome.tmp");
    let file = std::fs::File::create(&tmp_file)?;
    let mut writer = zip::ZipWriter::new(file);

    let mut files = Vec::new();
    collect_files_recursive(mod_dir, &mut files)?;

    for path in files {
        let rel_path = path
            .strip_prefix(mod_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let lower = rel_path.to_ascii_lowercase();
        let is_wad =
            lower.ends_with(".wad.client") || lower.ends_with(".wad") || lower.starts_with("wad/");

        let method = if is_wad {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };

        let options = zip::write::SimpleFileOptions::default().compression_method(method);

        writer.start_file(&rel_path, options)?;
        let mut f = std::fs::File::open(&path)?;
        std::io::copy(&mut f, &mut writer)?;
    }

    writer.finish()?;
    if staging_file.exists() {
        let _ = std::fs::remove_file(staging_file); // ignore-ok: replace previous staging archive
    }
    std::fs::rename(&tmp_file, staging_file)?;
    Ok(())
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_files_recursive(&entry.path(), out)?;
        } else if ft.is_file() {
            out.push(entry.path());
        }
    }
    Ok(())
}

pub struct LtfSession {
    client: BridgeClient,
    imported_mods: Arc<Mutex<Vec<String>>>,
    patcher_running: bool,
}

impl LtfSession {
    pub async fn start(
        tools_dirs: &[PathBuf],
        wait_timeout: Duration,
    ) -> Result<Self, InjectError> {
        let client = ensure_bridge_running(tools_dirs, wait_timeout).await?;
        wait_for_cdp_ready(&client, Duration::from_secs(25)).await?;
        Ok(Self {
            client,
            imported_mods: Arc::new(Mutex::new(Vec::new())),
            patcher_running: false,
        })
    }

    #[must_use]
    pub fn client(&self) -> &BridgeClient {
        &self.client
    }

    pub async fn apply_skin_archive(
        &mut self,
        archive_path: &Path,
        display_name: Option<&str>,
    ) -> Result<String, InjectError> {
        info!(archive = %archive_path.display(), "Importing skin into Mod Manager via bridge");
        let mod_id = self
            .client
            .import_archive(archive_path, display_name)
            .await?;

        if !mod_id.is_empty() {
            let mut list = self.imported_mods.lock().await;
            if !list.contains(&mod_id) {
                list.push(mod_id.clone());
            }
        }

        let target_id = if mod_id.is_empty() {
            display_name.unwrap_or("dekan_skin")
        } else {
            &mod_id
        };

        info!(mod_id = target_id, "Toggling mod active in Mod Manager");
        self.client.toggle_mod(target_id, true).await?;

        info!("Starting Mod Manager official patcher");
        self.client.start_patcher().await?;
        self.patcher_running = true;

        Ok(target_id.to_string())
    }

    pub async fn apply_mod_folder(
        &mut self,
        mod_dir: &Path,
        name: &str,
    ) -> Result<String, InjectError> {
        let staging_dir = dekan_platform::paths::data_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("Dekan"))
            .join("staging");
        let _ = std::fs::create_dir_all(&staging_dir); // ignore-ok: best-effort create
        let staging_file = staging_dir.join(format!("{name}.fantome"));
        pack_mod_fantome(mod_dir, &staging_file)?;
        self.apply_skin_archive(&staging_file, Some(name)).await
    }

    pub async fn stop(&mut self) -> Result<(), InjectError> {
        if self.patcher_running {
            info!("Stopping Mod Manager official patcher");
            let _ = self.client.stop_patcher().await; // ignore-ok: cleanup best-effort
            self.patcher_running = false;
        }

        let mut mods = self.imported_mods.lock().await;
        for mod_id in mods.drain(..) {
            info!(mod_id = %mod_id, "Removing temporary session mod from Mod Manager");
            let _ = self.client.toggle_mod(&mod_id, false).await; // ignore-ok: cleanup best-effort
            let _ = self.client.uninstall_mod(&mod_id).await; // ignore-ok: cleanup best-effort
        }

        Ok(())
    }
}

impl Drop for LtfSession {
    fn drop(&mut self) {
        if self.patcher_running {
            let client = self.client.clone();
            let mods = Arc::clone(&self.imported_mods);
            tokio::spawn(async move {
                let _ = client.stop_patcher().await; // ignore-ok: cleanup on drop
                let mut list = mods.lock().await;
                for mod_id in list.drain(..) {
                    let _ = client.toggle_mod(&mod_id, false).await; // ignore-ok: cleanup on drop
                    let _ = client.uninstall_mod(&mod_id).await; // ignore-ok: cleanup on drop
                }
            });
        }
    }
}

pub async fn spawn_ltf_patcher(
    tools_dirs: &[PathBuf],
    mods_dir: &Path,
    mods: &[String],
    state_tx: Option<StateSender>,
) -> Result<OverlayProcess, InjectError> {
    let _session_guard = LTF_SESSION_LOCK.lock().await;
    let client = ensure_bridge_running(tools_dirs, Duration::from_secs(30)).await?;
    wait_for_cdp_ready(&client, Duration::from_secs(25)).await?;
    let _ = client.health_check_off().await; // ignore-ok: best-effort disable health check

    let journal_path = journal::default_journal_path();
    journal::recover_leftovers(&client, &journal_path).await;

    {
        let mut stale_mods = ACTIVE_DEKAN_MODS.lock().await;
        let stale: Vec<String> = stale_mods.drain(..).collect();
        if !stale.is_empty() {
            journal::release_session(&client, &journal_path, &stale, &[]).await;
        } else if let Ok(st) = client.patcher_status().await {
            let (running, _, _) = super::client::extract_patcher_state(&st);
            if running {
                info!("Previous Mod Manager patcher is still running; waiting for it to stop");
                if !wait_patcher_stopped(&client, Duration::from_secs(15)).await {
                    warn!("Previous Mod Manager patcher did not report idle within 15s");
                }
            }
        }
    }

    let staging_dir = dekan_platform::paths::data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("Dekan"))
        .join("staging");
    let _ = std::fs::create_dir_all(&staging_dir); // ignore-ok: best-effort create

    let mut guard = journal::SessionCleanupGuard {
        client: client.clone(),
        journal_path: journal_path.clone(),
        imported: Vec::new(),
        disabled_others: Vec::new(),
        active: true,
    };

    for name in mods {
        let mod_dir = mods_dir.join(name);
        if !mod_dir.is_dir() {
            continue;
        }
        let staging_file = staging_dir.join(format!("{name}.fantome"));
        pack_mod_fantome(&mod_dir, &staging_file)?;

        let mod_id = match client.import_archive(&staging_file, Some(name)).await {
            Ok(id) => id,
            Err(first_err) => {
                warn!(error = %first_err, mod_name = %name, "Path import failed; retrying with byte transfer");
                let bytes = std::fs::read(&staging_file)?;
                client.import_bytes(&bytes, name).await?
            }
        };
        let target_id = if mod_id.is_empty() {
            name.clone()
        } else {
            mod_id
        };

        if !guard.imported.contains(&target_id) {
            guard.imported.push(target_id.clone());
            guard.persist();
        }
        client.toggle_mod(&target_id, true).await?;

        let mut active = ACTIVE_DEKAN_MODS.lock().await;
        if !active.contains(&target_id) {
            active.push(target_id);
        }
    }

    let keep_ids = guard.imported.clone();
    guard.disabled_others = disable_other_enabled_mods(&client, &keep_ids).await;
    guard.persist();

    let log_tail = ManagerLogTail::from_default_dir_at_end();
    info!("Starting Mod Manager patcher via LTF bridge");
    start_patcher_with_retry(&client).await?;

    let (imported_mods, disabled_others) = guard.disarm();
    let (tx, rx) = mpsc::unbounded_channel();
    let _ = tx.send(PatcherSignal::Armed); // ignore-ok: channel receiver is open

    let (stop_tx, stop_rx) = oneshot::channel();
    let monitor_task = monitor::spawn_monitor(monitor::MonitorContext {
        client: client.clone(),
        tx,
        log_tail,
        state_tx,
        stop_rx,
    });

    let stop_tx = Arc::new(std::sync::Mutex::new(Some(stop_tx)));
    let client_for_shutdown = client.clone();
    let shutdown_hook: crate::overlay_process::ShutdownHook = Box::new(move || {
        Box::pin(async move {
            if let Ok(mut slot) = stop_tx.lock() {
                if let Some(sender) = slot.take() {
                    let _ = sender.send(()); // ignore-ok: stop monitor loop
                }
            }
            let _guard = LTF_SESSION_LOCK.lock().await;
            journal::release_session(
                &client_for_shutdown,
                &journal_path,
                &imported_mods,
                &disabled_others,
            )
            .await;
            let mut active = ACTIVE_DEKAN_MODS.lock().await;
            active.retain(|id| !imported_mods.contains(id));
        })
    });

    Ok(OverlayProcess {
        child: None,
        _stdin: None,
        signals: rx,
        _stdout_reader: Some(monitor_task),
        _stderr_reader: None,
        hooked: false,
        shutdown_hook: Some(shutdown_hook),
    })
}

mod journal;
mod monitor;
mod ops;
use ops::{disable_other_enabled_mods, start_patcher_with_retry, wait_patcher_stopped};

#[cfg(test)]
mod tests;
