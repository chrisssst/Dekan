use super::*;

pub(super) fn measure_overlay(overlay_dir: &std::path::Path) -> (usize, u64) {
    measure_tree(overlay_dir, true)
}

pub(super) fn measure_tree(root: &std::path::Path, wad_only: bool) -> (usize, u64) {
    #[derive(Default)]
    struct Walk {
        files: usize,
        bytes: u64,
        failed: usize,
        first_error: Option<String>,
        wad_only: bool,
    }

    impl Walk {
        fn fail(&mut self, path: &std::path::Path, e: &std::io::Error) {
            self.failed += 1;
            self.first_error
                .get_or_insert_with(|| format!("{}: {e}", path.display()));
        }

        fn walk(&mut self, dir: &std::path::Path) {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) => return self.fail(dir, &e),
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(e) => {
                        self.fail(dir, &e);
                        continue;
                    }
                };
                let path = entry.path();
                match entry.file_type() {
                    Ok(ft) if ft.is_dir() => self.walk(&path),
                    Ok(ft) if ft.is_file() => {
                        let include = !self.wad_only
                            || path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| n.to_ascii_lowercase().ends_with(".wad.client"));
                        if include {
                            self.files += 1;
                            match entry.metadata() {
                                Ok(meta) => self.bytes += meta.len(),
                                Err(e) => self.fail(&path, &e),
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => self.fail(&path, &e),
                }
            }
        }
    }

    let mut walk = Walk {
        wad_only,
        ..Default::default()
    };
    walk.walk(root);
    if walk.failed > 0 {
        warn!(
            root = %root.display(),
            failed = walk.failed,
            first_error = walk.first_error.as_deref().unwrap_or_default(),
            "Overlay entries could not be measured"
        );
    }
    (walk.files, walk.bytes)
}

impl InjectionPipeline {
    pub(super) fn prepare_ltf_summary(&self, mods: &[String]) -> Result<OverlayBuild, InjectError> {
        let started = std::time::Instant::now();
        let mut file_count = 0usize;
        let mut total_bytes = 0u64;
        for name in mods {
            let mod_dir = self.config.overlay_config.mods_dir.join(name);
            if mod_dir.is_dir() {
                let (files, bytes) = measure_tree(&mod_dir, false);
                file_count += files;
                total_bytes += bytes;
            }
        }
        if file_count == 0 {
            return Err(InjectError::Overlay(
                "the selected mods contain no files to pack for Mod Manager".into(),
            ));
        }
        info!(
            mods = ?mods,
            files = file_count,
            bytes = total_bytes,
            "Prepared mods for Mod Manager; skipped native WAD overlay build"
        );
        Ok(OverlayBuild {
            wad_files: mods.len(),
            bytes: total_bytes,
            elapsed: started.elapsed(),
        })
    }

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
            Ok((build, 0)) => {
                self.record_overlay_cache(&effective_game_dir, mods);
                Ok(build)
            }
            Ok((build, stuck)) => {
                crate::overlay_cache::OverlayCache::invalidate(
                    &self.config.overlay_config.overlay_dir,
                );
                warn!(
                    stuck,
                    "Overlay cache not recorded: stray WADs from an earlier build are still in the overlay; the next match rebuilds it"
                );
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
            crate::overlay_cache::OverlayCache::invalidate(&self.config.overlay_config.overlay_dir);
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
    ) -> Result<(OverlayBuild, usize), InjectError> {
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
        Ok((
            OverlayBuild {
                wad_files,
                bytes,
                elapsed: native.elapsed,
            },
            native.stuck,
        ))
    }
}
