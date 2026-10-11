use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use dekan_app::catalog;

use crate::trigger;

pub fn resolve_library(paths: &mut trigger::ResolvedPaths) -> (std::path::PathBuf, bool) {
    paths.library_dir = catalog::resolve_library_root(&paths.library_dir);
    let mut library_root = paths.library_dir.clone();
    if let Err(e) = std::fs::create_dir_all(&library_root) {
        warn!(path = %library_root.display(), error = %e, "Could not create library root directory");
    }

    let local_library = dekan_platform::paths::data_dir()
        .map(|d| d.join("library"))
        .unwrap_or_else(|_| library_root.clone());

    let dir_has_content = |dir: &std::path::Path| -> bool {
        dir.is_dir()
            && std::fs::read_dir(dir)
                .map(|mut entries| entries.any(|e| e.is_ok()))
                .unwrap_or(false)
    };

    let mut library_has_content = dir_has_content(&library_root);
    if !library_has_content || (!dir_has_content(&local_library) && library_root != local_library) {
        let fallback_candidates = [
            paths.tools_dir.parent().map(|p| p.join("library")),
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|p| p.join("library"))),
            if library_has_content {
                Some(library_root.clone())
            } else {
                None
            },
        ];
        if let Some(source) = fallback_candidates
            .into_iter()
            .flatten()
            .find(|p| dir_has_content(p))
        {
            if source != local_library {
                info!(
                    source = %source.display(),
                    target = %local_library.display(),
                    "Seeding skin library into LocalAppData"
                );
                seed_directory(&source, &local_library);
            }
            if dir_has_content(&local_library) {
                paths.library_dir = local_library.clone();
                library_root = local_library;
                library_has_content = true;
            } else {
                warn!(
                    fallback = %source.display(),
                    "Could not seed LocalAppData library; falling back to read-only library source"
                );
                paths.library_dir = source.clone();
                library_root = source;
                library_has_content = true;
            }
        }
    }
    (library_root, library_has_content)
}

pub fn spawn_prewarm(
    paths: &trigger::ResolvedPaths,
    state_dir_path: &std::path::Path,
    state_rx: &dekan_core::state::StateReceiver,
    shutdown_token: &CancellationToken,
    custom_mods_notice: &dekan_app::mod_repair::CustomModsNotice,
    custom_mods_tx: std::sync::mpsc::Sender<dekan_app::mod_repair::ScanSummary>,
) {
    if paths.game_dir.is_dir() {
        dekan_inject::overlay_builder::persist_game_index_in(state_dir_path);
        let scan_notice = custom_mods_notice.clone();
        let gate_state = state_rx.clone();
        let gate_token = shutdown_token.clone();
        let game_dir = paths.game_dir.clone();
        let cache_dir = state_dir_path.to_path_buf();
        let mod_roots: Vec<std::path::PathBuf> = paths
            .mod_roots
            .iter()
            .map(|root| root.path.clone())
            .collect();
        let spawned = std::thread::Builder::new()
            .name("dekan-prewarm".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                match dekan_inject::overlay_builder::get_or_index_game(&game_dir) {
                    Ok(index) => info!(
                        wads = index.len(),
                        elapsed_ms = started.elapsed().as_millis(),
                        "Game WAD index ready"
                    ),
                    Err(e) => warn!(error = %e, "Game WAD index not prewarmed; the first build indexes on demand"),
                }
                let _background = dekan_platform::process::BackgroundThread::enter();
                let busy = || dekan_app::update_check::is_busy(gate_state.borrow().phase);
                while !gate_token.is_cancelled() && busy() {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                let report = dekan_app::mod_repair::repair_custom_mods(
                    &mod_roots,
                    &game_dir,
                    &cache_dir,
                    &|| gate_token.is_cancelled() || busy(),
                );
                if let Some(totals) = report.totals {
                    scan_notice.set(totals);
                }
                let _ = custom_mods_tx.send(report.this_run()); // ignore-ok: without a tray nobody listens, and the panel still reads the totals
                dekan_classic::generator::prewarm_companions(&game_dir, &cache_dir, || {
                    if gate_token.is_cancelled() {
                        dekan_classic::generator::PrewarmGate::Stop
                    } else if dekan_app::update_check::is_busy(gate_state.borrow().phase) {
                        dekan_classic::generator::PrewarmGate::Wait
                    } else {
                        dekan_classic::generator::PrewarmGate::Go
                    }
                });
            });
        if let Err(e) = spawned {
            warn!(error = %e, "Prewarm not started; the game is indexed and each champion scanned when needed");
        }
    } else {
        drop(custom_mods_tx);
    }
}

pub fn spawn_skin_sync(
    supervisor: &mut dekan_core::supervisor::Supervisor,
    library_root: &std::path::Path,
) {
    if let Some(sync_config) = dekan_app::skin_sync::SkinSyncConfig::from_env_value(
        std::env::var(dekan_core::env::SKIN_SYNC).ok().as_deref(),
    ) {
        info!(
            library = %library_root.display(),
            source = %sync_config.zip_url,
            "Skin library download enabled (DEKAN_SKIN_SYNC)"
        );
        let sync_lib_dir = library_root.to_path_buf();
        supervisor.spawn("skin-sync", move |child_token| async move {
            tokio::select! {
                _ = child_token.cancelled() => {}
                res = dekan_app::skin_sync::sync_skin_library(
                    &sync_lib_dir,
                    &sync_config,
                    false,
                ) => {
                    match res {
                        Ok(dekan_app::skin_sync::SkinSyncResult::Updated { sha, files_extracted }) => {
                            info!(
                                sha = %sha,
                                files = files_extracted,
                                "Skin library synchronized and updated from upstream repository"
                            );
                        }
                        Ok(dekan_app::skin_sync::SkinSyncResult::UpToDate { sha }) => {
                            debug!(sha = %sha, "Skin library is up to date with upstream repository");
                        }
                        Ok(dekan_app::skin_sync::SkinSyncResult::Skipped { reason }) => {
                            debug!(reason = %reason, "Skin library synchronization skipped");
                        }
                        Err(e) => {
                            warn!(error = %e, "Skin library sync failed; existing skins remain functional");
                        }
                    }
                }
            }
        });
    }
}

pub fn seed_directory(src: &std::path::Path, dst: &std::path::Path) {
    if !src.exists() {
        return;
    }

    // ignore-ok: best-effort creation of destination directory
    let _ = std::fs::create_dir_all(dst);
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let path = entry.path();
            let target = dst.join(entry.file_name());
            if path.is_dir() {
                seed_directory(&path, &target);
            } else if path.is_file()
                && !target.exists()
                && std::fs::hard_link(&path, &target).is_err()
            {
                // ignore-ok: fallback to file copy if hard link fails
                let _ = std::fs::copy(&path, &target);
            }
        }
    }
}
