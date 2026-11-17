use super::*;

impl OverlaySession {
    pub(super) fn warm_champion(&self, alias: Option<String>) {
        let Some(alias) = alias else {
            return;
        };
        let Some(game_dir) = dekan_platform::paths::normalize_game_dir(&self.mods.game_dir)
            .or_else(dekan_platform::paths::discover_game_dir)
        else {
            debug!("The game folder is not known; the champion is not warmed");
            return;
        };
        let cache_dir = self.mods.state_dir.clone();
        let state_rx = self.state_rx.clone();
        let turn = std::sync::Arc::clone(&self.warm_turn);
        let mine = turn.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
        drop(tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            if let Err(e) = dekan_classic::generator::StandardChampion::open(&game_dir, &alias)
                .map(|champion| champion.with_cache_dir(&cache_dir).companions())
            {
                debug!(error = %e, "Companion characters not indexed ahead of the build");
            }
            let skin_bin = dekan_wad::hash::wad_path_hash(&format!(
                "data/characters/{}/skins/skin0.bin",
                alias.to_ascii_lowercase()
            ));
            match dekan_inject::overlay_builder::read_wads_ahead(&game_dir, &[skin_bin], &|| {
                state_rx.borrow().phase.is_in_game()
                    || turn.load(std::sync::atomic::Ordering::Acquire) != mine
            }) {
                Ok(read) => info!(
                    alias = %alias,
                    files = read.files,
                    bytes = read.bytes,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Champion WAD read ahead; the patcher build finds it in the file cache"
                ),
                Err(dekan_inject::error::InjectError::Cancelled) => debug!(
                    alias = %alias,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Champion WAD read-ahead stopped: another champion took its place or the match started"
                ),
                Err(e) => {
                    warn!(alias = %alias, error = %e, "Champion WAD not read ahead; the patcher build reads it from the disk")
                }
            }
        }));
    }

    pub(super) async fn refresh_mods(
        &mut self,
        champion_id: ChampionId,
        alias: Option<String>,
    ) -> ModsPanel {
        if alias.is_none() {
            debug!(
                champion_id,
                "No WAD alias from the client; skin mods outside a champion folder are not offered"
            );
        }
        self.mod_catalog =
            mods_store::load_mod_catalog(self.mods.roots.clone(), Some(champion_id), alias).await;

        let mut selection = self.state_rx.borrow().mods.clone();
        let dropped = selection.prune(&self.mod_catalog, Some(champion_id));
        if !dropped.is_empty() {
            info!(
                dropped = ?dropped,
                "Selected custom mods are no longer on disk; removed from the selection"
            );
            mods_store::save_selection(&self.mods.state_dir, &selection);
            set_mod_selection(&self.state_tx, selection.clone());
        }

        ModsPanel {
            available: self.mod_catalog.clone(),
            selection: selection.view(Some(champion_id)),
        }
    }

    pub(super) fn custom_skin_selected(&self, champion_id: ChampionId) -> bool {
        self.state_rx.borrow().mods.skin.contains_key(&champion_id)
    }

    pub(super) fn apply_mod_request(
        &mut self,
        request: &dekan_core::mods::ModSelectionView,
        champion: Option<ChampionId>,
    ) {
        let current = self.state_rx.borrow().mods.clone();
        let (next, rejected) = self.mod_catalog.apply_request(&current, champion, request);

        for refused in &rejected {
            warn!(mod_id = %refused.id, reason = refused.reason, "Custom mod selection refused");
        }

        if next != current {
            info!(
                champion_id = ?champion,
                skin_mod = ?champion.and_then(|c| next.skin.get(&c)),
                map = ?next.map,
                font = ?next.font,
                announcer = ?next.announcer,
                others = ?next.others,
                "Custom mod selection changed"
            );
            mods_store::save_selection(&self.mods.state_dir, &next);
            set_mod_selection(&self.state_tx, next.clone());
        }

        let custom_now = champion.is_some_and(|c| next.skin.contains_key(&c));
        let restored_here = self
            .historic_restored
            .as_ref()
            .is_some_and(|restored| Some(restored.champion_id) == champion);
        if custom_now && restored_here {
            info!(
                champion_id = ?champion,
                "A custom skin was selected; the saved skin restored for this champion is dropped"
            );
            self.historic_restored = None;
            clear_overlay_target(&self.state_tx);
            self.show_selection(None, None);
        }

        self.controller.set_mod_selection(next.view(champion));
    }
}
