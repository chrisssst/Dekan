use super::*;

pub(super) fn prefer_lazy_wad_checks(game_dir: &Path) {
    match dekan_platform::client_settings::disable_crash_reporting(game_dir) {
        Ok(true) => info!(
            "Turned the League client's crash reporting off so the injector checks archives as the game loads them"
        ),
        Ok(false) => debug!(
            "The League client's crash reporting is already off or the client has no settings yet"
        ),
        Err(e) => warn!(
            error = %e,
            "Could not turn the League client's crash reporting off; the injector checks every archive as the match starts"
        ),
    }
}

pub(super) async fn match_ended(state_rx: &mut StateReceiver) {
    loop {
        if state_rx.borrow_and_update().phase.is_between_matches() {
            return;
        }
        if state_rx.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

pub(super) fn note_wanted(last: &mut Option<WantedSkin>, wanted: WantedSkin, phase: GamePhase) {
    if *last != Some(wanted) {
        debug!(phase = ?phase, wanted = ?wanted, "The skin to arm for changed");
        *last = Some(wanted);
    }
}

impl InjectionTrigger {
    pub(super) fn effective_game_dir(&self, pid: Option<u32>) -> PathBuf {
        let from_process = pid.and_then(|pid| match ProcessFinder::get_process_path(pid) {
            Ok(Some(exe_path)) => exe_path
                .parent()
                .and_then(dekan_platform::paths::normalize_game_dir),
            Ok(None) => None,
            Err(e) => {
                debug!(pid, error = %e, "The game process path could not be read; using the known game folder");
                None
            }
        });

        let dir = from_process
            .or_else(|| dekan_platform::paths::normalize_game_dir(&self.paths.game_dir))
            .or_else(dekan_platform::paths::discover_game_dir)
            .unwrap_or_else(|| self.paths.game_dir.clone());
        if dir.is_dir() {
            dekan_inject::overlay_builder::prewarm_game_index(&dir);
        } else {
            warn!(game = %dir.display(), "The game folder is not known or does not exist; the overlay build may fail");
        }
        dir
    }

    pub(super) fn pipeline_config(&self, game_dir: PathBuf) -> PipelineConfig {
        PipelineConfig {
            provider_kind: dekan_inject::provider::ProviderKind::Ltf,
            native_host_exe: None,
            native_dll_path: None,
            native_target: None,
            overlay_config: OverlayConfig {
                mods_dir: self.paths.mods_dir.clone(),
                overlay_dir: self.paths.overlay_dir.clone(),
                game_dir,
            },
            hook_timeout: dekan_inject::pipeline::DEFAULT_HOOK_TIMEOUT,
            build_timeout: dekan_inject::pipeline::DEFAULT_BUILD_TIMEOUT,
            late_budget: dekan_inject::pipeline::DEFAULT_LATE_BUDGET,
        }
    }
}
