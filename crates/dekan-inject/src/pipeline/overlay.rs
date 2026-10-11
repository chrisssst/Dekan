use super::*;

pub(super) fn measure_overlay(overlay_dir: &std::path::Path) -> (usize, u64) {
    fn walk(dir: &std::path::Path, files: &mut usize, bytes: &mut u64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => walk(&path, files, bytes),
                Ok(ft) if ft.is_file() => {
                    let is_wad = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".wad.client"));
                    if is_wad {
                        *files += 1;
                        *bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                    }
                }
                _ => {}
            }
        }
    }

    let mut files = 0usize;
    let mut bytes = 0u64;
    walk(overlay_dir, &mut files, &mut bytes);
    (files, bytes)
}

impl InjectionPipeline {
    pub(super) async fn build_overlay(
        &self,
        mods: &[String],
        timeout: Duration,
    ) -> Result<OverlayBuild, InjectError> {
        OverlayManager::prepare_overlay_dir(&self.config.overlay_config.overlay_dir)?;

        let effective_game_dir =
            dekan_platform::paths::normalize_game_dir(&self.config.overlay_config.game_dir)
                .unwrap_or_else(|| self.config.overlay_config.game_dir.clone());

        if let Some((wad_files, bytes)) = crate::overlay_cache::OverlayCache::is_fresh(
            &effective_game_dir,
            &self.config.overlay_config.mods_dir,
            &self.config.overlay_config.overlay_dir,
            mods,
        ) {
            info!(
                mods = ?mods,
                wad_files,
                overlay_bytes = bytes,
                "Overlay cache hit: fingerprint and base game WADs unchanged; skipping build"
            );
            return Ok(OverlayBuild {
                wad_files,
                bytes,
                elapsed: Duration::ZERO,
            });
        }

        match self.build_overlay_native(mods, timeout).await {
            Ok(build) => {
                self.record_overlay_cache(&effective_game_dir, mods);
                Ok(build)
            }
            Err(e) => {
                crate::overlay_cache::OverlayCache::invalidate(
                    &self.config.overlay_config.overlay_dir,
                );
                if !matches!(e, InjectError::Cancelled) {
                    warn!(error = %e, mods = ?mods, budget_ms = timeout.as_millis(), "Native overlay build failed");
                }
                Err(e)
            }
        }
    }

    pub(super) fn record_overlay_cache(&self, game_dir: &std::path::Path, mods: &[String]) {
        if let Err(e) = crate::overlay_cache::OverlayCache::record(
            game_dir,
            &self.config.overlay_config.mods_dir,
            &self.config.overlay_config.overlay_dir,
            mods,
        ) {
            warn!(
                error = %e,
                "Overlay cache fingerprint not recorded; the next match rebuilds this overlay"
            );
        }
    }

    pub(super) async fn build_overlay_native(
        &self,
        mods: &[String],
        timeout: Duration,
    ) -> Result<OverlayBuild, InjectError> {
        let config = &self.config.overlay_config;
        let game_dir = dekan_platform::paths::normalize_game_dir(&config.game_dir)
            .unwrap_or_else(|| config.game_dir.clone());
        let (mods_dir, overlay_dir) = (config.mods_dir.clone(), config.overlay_dir.clone());
        let mods = mods.to_vec();

        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _cancel_on_drop = CancelOnDrop(cancel.clone());
        let flag = cancel.clone();
        let task = tokio::task::spawn_blocking(move || {
            crate::overlay_builder::build(&game_dir, &mods_dir, &overlay_dir, &mods, &flag)
        });
        let native = match tokio::time::timeout(timeout, task).await {
            Ok(Ok(result)) => result?,
            Ok(Err(join)) => {
                return Err(InjectError::Overlay(format!(
                    "native overlay task failed: {join}"
                )));
            }
            Err(_) => {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                return Err(InjectError::SubprocessTimeout {
                    command: "overlay build".into(),
                    timeout_secs: timeout.as_secs(),
                });
            }
        };

        let (wad_files, bytes) = measure_overlay(&self.config.overlay_config.overlay_dir);
        if wad_files == 0 {
            return Err(InjectError::Overlay(
                "the overlay build produced an empty overlay".into(),
            ));
        }
        info!(
            wad_files,
            rewritten = native.written,
            overlay_bytes = bytes,
            elapsed_ms = native.elapsed.as_millis(),
            "Overlay built natively"
        );
        Ok(OverlayBuild {
            wad_files,
            bytes,
            elapsed: native.elapsed,
        })
    }
}
