use super::*;

pub(crate) struct ArmedPatcher {
    pub(crate) overlay: dekan_inject::overlay_process::OverlayProcess,
    pub(crate) key: ArmKey,

    pub(crate) armed_before_game: bool,

    pub(crate) lcu_skin: Option<u32>,
}

pub(crate) struct Arming<'a> {
    pub(crate) key: ArmKey,
    pub(crate) task:
        std::pin::Pin<Box<dyn std::future::Future<Output = Option<ArmedPatcher>> + Send + 'a>>,
}

pub(crate) fn resume_for_reconnect(
    running: Option<dekan_inject::overlay_process::OverlayProcess>,
    wanted: WantedSkin,
) -> (
    Option<ArmedPatcher>,
    Option<dekan_inject::overlay_process::OverlayProcess>,
) {
    match (running, wanted) {
        (Some(overlay), WantedSkin::Skin(key)) => (
            Some(ArmedPatcher {
                overlay,
                key,
                armed_before_game: true,
                lcu_skin: None,
            }),
            None,
        ),
        (running, _) => (None, running),
    }
}

pub(crate) async fn next_armed(arming: &mut Option<Arming<'_>>) -> Option<ArmedPatcher> {
    match arming {
        Some(in_flight) => in_flight.task.as_mut().await,
        None => std::future::pending().await,
    }
}

impl InjectionTrigger {
    pub(crate) async fn arm_patcher(&self, request: ArmRequest) -> Option<ArmedPatcher> {
        let key = request.key();
        let ArmKey {
            champ_id,
            entry_id,
            mods,
            second,
            lobby,
            ..
        } = key;

        info!(
            champ_id,
            entry_id = ?entry_id,
            second = ?second,
            lobby,
            mods_fingerprint = mods,
            "Preparing the overlay for the chosen skin and mods, before the game starts"
        );

        let registration = async {
            match entry_id {
                Some(_) if is_classic(champ_id) => None,
                Some(_) if self.state_rx.borrow().phase == GamePhase::Reconnect => {
                    debug!(
                        champ_id,
                        entry_id = ?entry_id,
                        "Reconnect: the skin is not registered in the client"
                    );
                    None
                }
                Some(_) if lobby => {
                    self.register_in_lobby(&key.picks()).await;
                    None
                }
                Some(entry_id) => self.register_in_champ_select(champ_id, entry_id).await,
                None => None,
            }
        };
        let (armed, lcu_skin) = tokio::join!(self.build_and_arm(key), registration);
        let (overlay, build) = armed?;

        let game_already_running = match ProcessFinder::find_any_process(
            &dekan_platform::game_version::GAME_EXES,
        ) {
            Ok(found) => found.is_some(),
            Err(e) => {
                warn!(error = %e, "The game process could not be looked up; assuming it is not running yet");
                false
            }
        };
        if game_already_running {
            warn!(
                champ_id,
                entry_id = ?entry_id,
                build_ms = build.elapsed.as_millis(),
                "Patcher armed after the game process already existed; the hook may land too late for the skin to load"
            );
        } else {
            info!(
                champ_id,
                entry_id = ?entry_id,
                wad_files = build.wad_files,
                lcu_skin = ?lcu_skin,
                "Patcher armed; the game will be hooked as soon as it starts"
            );
        }
        Some(ArmedPatcher {
            overlay,
            key,
            armed_before_game: !game_already_running,
            lcu_skin,
        })
    }

    pub(crate) async fn build_and_arm(
        &self,
        key: ArmKey,
    ) -> Option<(
        dekan_inject::overlay_process::OverlayProcess,
        dekan_inject::pipeline::OverlayBuild,
    )> {
        let mods = self.collect_mods(key).await?;
        let game_dir = self.effective_game_dir(None);
        if dekan_platform::preferences::LIGHT_LOADING.is_enabled() {
            prefer_lazy_wad_checks(&game_dir);
        }
        let pipeline =
            InjectionPipeline::new(self.pipeline_config(game_dir), Some(self.state_tx.clone()));

        match pipeline.arm(&mods).await {
            Ok(armed) => Some(armed),
            Err(e) => {
                warn!(
                    error = %e,
                    champ_id = key.champ_id,
                    entry_id = ?key.entry_id,
                    "Could not arm the patcher; the game-start path will retry against the running game"
                );
                None
            }
        }
    }

    pub(crate) async fn reread_selection(
        &self,
        state: &dekan_core::state::AppState,
    ) -> (Option<u32>, Option<u32>) {
        let from_state = (state.champion_id, state.selected_skin_id);

        let discovery = dekan_lcu::client::LcuClient::discover().await;
        let client = match discovery {
            Ok(client) => client,
            Err(e) => {
                warn!(error = %e, "Rule #1 re-read skipped: no LCU client; using the published state");
                return from_state;
            }
        };

        match dekan_lcu::live_selection::resolve_live_selection(&client, state.champion_id).await {
            Ok(live) => {
                if from_state != (Some(live.champion_id), Some(live.skin_id)) {
                    warn!(
                        state_champion = ?state.champion_id,
                        state_skin = ?state.selected_skin_id,
                        live_champion = live.champion_id,
                        live_skin = live.skin_id,
                        source = ?live.source,
                        "Live selection differs from the published state; the live value wins"
                    );
                } else {
                    info!(
                        champion_id = live.champion_id,
                        skin_id = live.skin_id,
                        source = ?live.source,
                        "Selection confirmed against the LCU before injecting"
                    );
                }
                (Some(live.champion_id), Some(live.skin_id))
            }
            Err(e) => {
                warn!(
                    error = %e,
                    state_champion = ?state.champion_id,
                    state_skin = ?state.selected_skin_id,
                    "Rule #1 re-read failed; falling back to the published state"
                );
                from_state
            }
        }
    }

    pub(crate) async fn register_in_champ_select(
        &self,
        champ_id: u32,
        entry_id: u32,
    ) -> Option<u32> {
        let client = self.lcu_client().await?;

        let owned = match client.get_owned_skin_ids().await {
            Ok(owned) => owned,
            Err(e) => {
                warn!(
                    error = %e,
                    champ_id,
                    "Owned skins unavailable; registering the base skin"
                );
                std::collections::HashSet::new()
            }
        };

        let parent = parent_skin(&client, champ_id, entry_id).await;
        let skin_id =
            dekan_lcu::skin_registration::skin_to_register(champ_id, entry_id, parent, &owned);
        info!(
            champ_id,
            entry_id,
            parent = ?parent,
            skin_id,
            owned = skin_id == entry_id,
            owned_skins = owned.len(),
            "Registering the skin in champ select"
        );

        match dekan_lcu::skin_registration::register_skin(&client, skin_id).await {
            dekan_lcu::skin_registration::RegistrationOutcome::Verified { .. } => Some(skin_id),
            _ => None,
        }
    }

    pub(crate) async fn register_in_lobby(&self, picks: &[(u32, u32)]) {
        let Some(client) = self.lcu_client().await else {
            return;
        };
        let owned = match client.get_owned_skin_ids().await {
            Ok(owned) => owned,
            Err(e) => {
                warn!(error = %e, "Owned skins unavailable; registering the base skins in the lobby");
                std::collections::HashSet::new()
            }
        };
        let mut skins: Vec<(u32, u32)> = Vec::with_capacity(picks.len());
        for &(champ_id, entry_id) in picks {
            let parent = parent_skin(&client, champ_id, entry_id).await;
            skins.push((
                champ_id,
                dekan_lcu::skin_registration::skin_to_register(champ_id, entry_id, parent, &owned),
            ));
        }
        if let Err(e) = client.set_lobby_slot_skins(&skins).await {
            warn!(error = %e, skins = ?skins, "Could not register the skins in the lobby slots");
        }
    }

    pub(crate) async fn lcu_client(&self) -> Option<dekan_lcu::client::LcuClient> {
        dekan_lcu::client::LcuClient::discover()
            .await
            .inspect_err(|e| warn!(error = %e, "No LCU client; the skin cannot be registered in champ select"))
            .ok()
    }

    pub(crate) async fn release(armed: Option<ArmedPatcher>) {
        if let Some(armed) = armed {
            armed.overlay.shutdown().await;
        }
    }
}

async fn parent_skin(
    client: &dekan_lcu::client::LcuClient,
    champ_id: u32,
    entry_id: u32,
) -> Option<u32> {
    if dekan_core::selection::is_base_skin(entry_id, champ_id) {
        return None;
    }
    client
        .get_champion_assets(champ_id)
        .await
        .inspect_err(|e| {
            debug!(
                champ_id,
                entry_id,
                error = %e,
                "Champion assets unavailable; the parent skin of the entry is not checked"
            );
        })
        .ok()?
        .base_skin_of(entry_id)
}
