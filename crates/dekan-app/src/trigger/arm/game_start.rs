use super::*;

impl InjectionTrigger {
    pub(crate) async fn handle_game_start(
        &self,
        state: &dekan_core::state::AppState,
        armed: Option<ArmedPatcher>,
        token: &CancellationToken,
    ) -> Option<dekan_inject::overlay_process::OverlayProcess> {
        let started_at = tokio::time::Instant::now();

        let (champion_id, live_skin_id) = self.reread_selection(state).await;

        let (target, mods, party, lobby) = {
            let state = self.state_rx.borrow();
            let (party, _) = party_skins(&state);
            (
                state.overlay_target.clone(),
                state.mods.clone(),
                dekan_core::party::party_fingerprint(&party),
                state.lobby.clone(),
            )
        };

        let Some(champ_id) = champion_id else {
            if target.is_none() && mods.fingerprint(None) == 0 {
                info!(
                    live_skin_id = ?live_skin_id,
                    "No skin chosen in the overlay; nothing to inject this match"
                );
            } else {
                warn!(
                    target_champion = ?target.as_ref().map(|t| t.champion_id),
                    "Refusing to inject: the live champion is unknown, so the target cannot be validated"
                );
            }
            Self::release(armed).await;
            return None;
        };

        let armed_in_lobby = armed
            .as_ref()
            .map(|a| a.key)
            .filter(|key| key.lobby && key.covers(champ_id));
        if let Some(armed_key) = armed_in_lobby {
            info!(
                champ_id,
                entry_id = ?armed_key.entry_for(champ_id),
                armed_for = ?armed_key.picks(),
                "Injecting the skin picked in the lobby for the champion this match gave"
            );
            dekan_core::state::focus_lobby_champion(&self.state_tx, champ_id);
            if let Some(armed) = armed {
                return self.confirm_armed(armed, champ_id, started_at).await;
            }
        }

        let target = match &lobby {
            Some(lobby) => lobby.target_for(champ_id).cloned().or(target),
            None => target,
        };

        let entry_id = match &target {
            None => None,
            Some(target) if !target.matches_champion(champ_id) => {
                warn!(
                    target_champion = target.champion_id,
                    live_champion = champ_id,
                    "Refusing to inject: the chosen skin belongs to another champion"
                );
                None
            }
            Some(target) => {
                let entry_id = target.package_entry_id();
                if dekan_core::selection::is_base_skin(entry_id, champ_id) {
                    info!(
                        champ_id,
                        entry_id, "Base skin chosen; there is no skin to overlay"
                    );
                    None
                } else {
                    Some(entry_id)
                }
            }
        };

        let Some(key) = build_key(champ_id, entry_id, &mods, live_skin_id, party) else {
            info!(
                champ_id,
                live_skin_id = ?live_skin_id,
                "Nothing to inject this match: no skin chosen and no custom mod selected"
            );
            Self::release(armed).await;
            return None;
        };

        info!(
            champ_id,
            skin_id = ?target.as_ref().map(|t| t.skin_id),
            chroma_id = ?target.as_ref().and_then(|t| t.chroma_id),
            entry_id = ?entry_id,
            custom_mods = ?mods.ordered_ids(Some(champ_id)),
            classic = is_classic(champ_id),
            live_skin_id = ?live_skin_id,
            "Injecting the skin chosen in the overlay"
        );

        if let Some(armed) = armed {
            if armed.key == key {
                return self.confirm_armed(armed, champ_id, started_at).await;
            }

            warn!(
                armed_entry = ?armed.key.entry_id,
                entry_id = ?entry_id,
                armed_mods = armed.key.mods,
                mods = key.mods,
                "The armed patcher was built for another selection; rebuilding against the running game"
            );
            armed.overlay.shutdown().await;
        }

        self.inject_late(key, started_at, token).await
    }

    pub(crate) async fn confirm_armed(
        &self,
        mut armed: ArmedPatcher,
        champ_id: u32,
        started_at: tokio::time::Instant,
    ) -> Option<dekan_inject::overlay_process::OverlayProcess> {
        let pipeline = InjectionPipeline::new(
            self.pipeline_config(self.effective_game_dir(None)),
            Some(self.state_tx.clone()),
        );

        let status = pipeline
            .confirm_hook(
                &mut armed.overlay,
                dekan_inject::pipeline::DEFAULT_HOOK_TIMEOUT,
            )
            .await;

        info!(
            champ_id,
            entry_id = ?armed.key.entry_for(champ_id),
            status = ?status,
            armed_before_game = armed.armed_before_game,
            registered_lcu_skin = ?armed.lcu_skin,
            hook_ms = started_at.elapsed().as_millis(),
            "Injection completed from the pre-armed patcher"
        );
        set_injection_status(&self.state_tx, status);
        Some(armed.overlay)
    }

    pub(crate) async fn inject_late(
        &self,
        key: ArmKey,
        started_at: tokio::time::Instant,
        token: &CancellationToken,
    ) -> Option<dekan_inject::overlay_process::OverlayProcess> {
        let ArmKey {
            champ_id, entry_id, ..
        } = key;
        warn!(
            champ_id,
            entry_id = ?entry_id,
            "No patcher was armed for this match; building the overlay against the running game. \
             The hook may land after the game has already read its WADs, in which case the stock \
             skin is what loads"
        );

        let mods = self.collect_mods(key).await?;

        let mut game_pid = None;
        let discovery_started = tokio::time::Instant::now();
        let max_wait = Duration::from_secs(60);

        while discovery_started.elapsed() < max_wait && !token.is_cancelled() {
            let found = tokio::task::spawn_blocking(|| {
                ProcessFinder::find_any_process(&dekan_platform::game_version::GAME_EXES)
            })
            .await;
            if let Ok(Ok(Some(pid))) = found {
                game_pid = Some(pid);
                break;
            }
            tokio::time::sleep(GAME_PROCESS_POLL).await;
        }

        let pid = match game_pid {
            Some(pid) => pid,
            None => {
                warn!("Timed out waiting for the game process to spawn");
                set_injection_status(
                    &self.state_tx,
                    InjectionStatus::Failed {
                        error: "Game process not detected within 60s timeout".into(),
                    },
                );
                return None;
            }
        };

        info!(
            pid,
            discovery_ms = discovery_started.elapsed().as_millis(),
            "Target League game process discovered; executing injection pipeline"
        );

        let pipeline = InjectionPipeline::new(
            self.pipeline_config(self.effective_game_dir(Some(pid))),
            Some(self.state_tx.clone()),
        );

        debug!(
            champ_id,
            entry_id = ?entry_id,
            "Late path: the loading-screen card can no longer be changed"
        );
        let outcome = pipeline.execute(&mods, pid).await;

        match outcome {
            Ok(outcome) => {
                info!(
                    status = ?outcome.status,
                    hook_ms = started_at.elapsed().as_millis(),
                    "Injection sequence completed"
                );
                outcome.overlay
            }
            Err(e) => {
                error!(error = %e, "Injection sequence failed");
                None
            }
        }
    }
}
