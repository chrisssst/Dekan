use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomFallback {
    pub enabled: bool,
    pub finalization: bool,
    pub target_chosen: bool,
    pub already_rolled: bool,
    pub declined: bool,
    pub lcu_skin: Option<dekan_core::selection::SkinId>,
}

#[must_use]
pub fn should_roll_random(champion_id: ChampionId, fallback: RandomFallback) -> bool {
    fallback.enabled
        && fallback.finalization
        && !fallback.target_chosen
        && !fallback.already_rolled
        && !fallback.declined
        && fallback
            .lcu_skin
            .is_none_or(|skin| dekan_core::selection::is_base_skin(skin, champion_id))
}

pub(super) fn random_index(bound: usize) -> usize {
    use std::hash::{BuildHasher, Hasher};
    let value = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    (value % bound.max(1) as u64) as usize
}

pub(super) fn handle_command(
    state_tx: &StateSender,
    command: OverlayCommand,
    catalog: Option<&Catalog>,
) {
    match command {
        OverlayCommand::Select { id } => {
            let Some(catalog) = catalog else {
                warn!(
                    entry_id = id,
                    "Selection arrived with no catalog loaded; ignoring it"
                );
                return;
            };

            match catalog.resolve_target(id) {
                Some(target) => {
                    info!(
                        champion_id = target.champion_id,
                        skin_id = target.skin_id,
                        chroma_id = ?target.chroma_id,
                        package_entry = target.package_entry_id(),
                        "Injection target chosen in the overlay"
                    );
                    set_overlay_target(state_tx, target);
                }

                None => {
                    warn!(
                        entry_id = id,
                        champion_id = catalog.champion_id,
                        "Selected entry is not in this champion's catalog; refusing to guess"
                    );
                }
            }
        }
        OverlayCommand::Clear => {
            debug!("Selection cleared in the overlay");
            clear_overlay_target(state_tx);
        }

        OverlayCommand::SetMods { .. }
        | OverlayCommand::OpenModsFolder
        | OverlayCommand::ImportMod { .. }
        | OverlayCommand::ChromaPreview { .. }
        | OverlayCommand::FocusChampion { .. }
        | OverlayCommand::TogglePreset
        | OverlayCommand::SetProfile { .. }
        | OverlayCommand::NewProfile
        | OverlayCommand::DeleteProfile
        | OverlayCommand::Random => {
            warn!("Session command reached the skin handler; ignoring it");
        }
    }
}

impl OverlaySession {
    pub(super) fn track_historic(
        &mut self,
        champion_id: ChampionId,
        catalog: &Catalog,
        target: Option<&OverlayTarget>,
        lcu_skin: Option<dekan_core::selection::SkinId>,
    ) {
        if let Some(restored) = self.historic_restored.clone() {
            if target != Some(&restored) {
                self.historic_restored = None;
            } else if historic::superseded_in_client(champion_id, lcu_skin) {
                info!(
                    champion_id,
                    lcu_skin = ?lcu_skin,
                    entry_id = restored.package_entry_id(),
                    "An owned skin was picked in the client; the restored historic skin is dropped"
                );
                self.historic_restored = None;
                clear_overlay_target(&self.state_tx);
                self.show_selection(None, None);
            }
            return;
        }

        if self.historic_consulted == Some(champion_id) {
            return;
        }
        self.historic_consulted = Some(champion_id);

        let Some((entry, origin)) = self.saved_skin(champion_id) else {
            return;
        };
        if !historic::may_restore(
            champion_id,
            target.is_some(),
            self.custom_skin_selected(champion_id),
            lcu_skin,
        ) {
            debug!(
                champion_id,
                lcu_skin = ?lcu_skin,
                chosen = target.is_some(),
                origin = ?origin,
                "Saved skin not restored: a choice is already made"
            );
            return;
        }

        let Some(restored) = catalog.resolve_target(entry.package_entry_id()) else {
            info!(
                champion_id,
                entry_id = entry.package_entry_id(),
                origin = ?origin,
                "Saved skin is no longer in the catalog; not restored"
            );
            return;
        };
        info!(
            champion_id,
            skin_id = restored.skin_id,
            chroma_id = ?restored.chroma_id,
            origin = ?origin,
            profile = self.presets.active(),
            "Saved skin restored as the injection target"
        );
        set_overlay_target(&self.state_tx, restored.clone());
        self.show_selection(Some(restored.package_entry_id()), Some(origin));
        self.restored_from_preset = origin == SelectionOrigin::Preset;
        self.historic_restored = Some(restored);
    }

    pub(super) fn saved_skin(
        &self,
        champion_id: ChampionId,
    ) -> Option<(dekan_core::historic::HistoricEntry, SelectionOrigin)> {
        if self.declined_presets.contains(&champion_id) {
            return None;
        }
        self.presets
            .preset(champion_id)
            .map(|entry| (entry, SelectionOrigin::Preset))
            .or_else(|| {
                self.historic
                    .get(champion_id)
                    .map(|entry| (entry, SelectionOrigin::Historic))
            })
    }

    pub(super) fn restore_lobby_champions(
        &mut self,
        lobby: &[ChampionId],
        focused: Option<ChampionId>,
    ) {
        for &champion_id in lobby {
            if Some(champion_id) == focused || !self.lobby_restored.insert(champion_id) {
                continue;
            }
            let (chosen, slot_skin) = {
                let state = self.state_rx.borrow();
                let lobby = state.lobby.as_ref();
                (
                    lobby.and_then(|l| l.target_for(champion_id)).is_some(),
                    lobby.and_then(|l| l.skin_of(champion_id)),
                )
            };
            let Some((entry, origin)) = self.saved_skin(champion_id) else {
                continue;
            };
            if !historic::may_restore(
                champion_id,
                chosen,
                self.custom_skin_selected(champion_id),
                slot_skin,
            ) {
                continue;
            }
            let target = OverlayTarget {
                champion_id,
                skin_id: entry.skin_id,
                chroma_id: entry.chroma_id,
            };
            if set_lobby_target(&self.state_tx, &target) {
                self.lobby_auto.insert(champion_id);
                info!(
                    champion_id,
                    entry_id = target.package_entry_id(),
                    origin = ?origin,
                    "Saved skin restored for the lobby's other champion"
                );
            }
        }
    }

    pub(super) fn publish_presets(&self, champion: Option<ChampionId>) {
        let profiles = self.presets.profiles();
        let active = profiles
            .iter()
            .position(|name| name == self.presets.active())
            .unwrap_or(0);
        self.controller.set_presets(PresetsView {
            profiles,
            active,
            preset_entry: champion
                .and_then(|champion| self.presets.preset(champion))
                .map(|entry| entry.package_entry_id()),
        });
    }

    pub(super) fn toggle_preset(&mut self, champion: Option<ChampionId>) {
        let target = self.state_rx.borrow().overlay_target.clone();
        let Some(target) = target.filter(|t| Some(t.champion_id) == champion) else {
            info!(champion_id = ?champion, "Preset asked with no skin chosen for this champion; nothing pinned");
            return;
        };
        let pinned = self.presets.toggle(&target);
        info!(
            champion_id = target.champion_id,
            entry_id = target.package_entry_id(),
            pinned,
            profile = self.presets.active(),
            "Skin preset changed"
        );
        preset_store::save(&self.mods.state_dir, &self.presets);
        self.publish_presets(champion);
    }

    pub(super) fn new_profile(&mut self, locale: Option<&str>, champion: Option<ChampionId>) {
        let base = dekan_platform::i18n::Language::for_locale(locale)
            .text()
            .overlay_profile_base;
        let name = self.presets.create(base);
        info!(profile = %name, "Skin profile created and switched to");
        self.profile_changed(champion);
    }

    pub(super) fn profile_changed(&mut self, champion: Option<ChampionId>) {
        preset_store::save(&self.mods.state_dir, &self.presets);
        if self.historic_restored.take().is_some() {
            clear_overlay_target(&self.state_tx);
            self.show_selection(None, None);
        }
        for champion_id in self.lobby_auto.drain() {
            clear_lobby_target(&self.state_tx, champion_id);
        }
        self.historic_consulted = None;
        self.lobby_restored.clear();
        self.publish_presets(champion);
    }

    pub(super) fn record_historic(&mut self, confirmed: bool, target: Option<&OverlayTarget>) {
        if !confirmed {
            self.historic_recorded = None;
            return;
        }
        let Some(target) = target else {
            return;
        };
        if self.historic_recorded.as_ref() == Some(target) {
            return;
        }
        self.historic_recorded = Some(target.clone());
        if self.historic.record(target) {
            info!(
                champion_id = target.champion_id,
                skin_id = target.skin_id,
                chroma_id = ?target.chroma_id,
                "Historic skin recorded for the champion"
            );
            historic_store::save(&self.mods.state_dir, &self.historic);
        }
    }

    pub(super) fn dismiss_historic(&mut self) {
        let Some(restored) = self.historic_restored.take() else {
            return;
        };
        if std::mem::take(&mut self.restored_from_preset) {
            self.declined_presets.insert(restored.champion_id);
            info!(
                champion_id = restored.champion_id,
                "Restored preset cleared; it is not restored again until this selection ends"
            );
            return;
        }

        if self.state_rx.borrow().overlay_target.as_ref() != Some(&restored) {
            return;
        }
        if self.historic.forget(restored.champion_id) {
            info!(
                champion_id = restored.champion_id,
                "Historic skin dismissed; it will not be restored again for this champion"
            );
            historic_store::save(&self.mods.state_dir, &self.historic);
        }
    }

    pub(super) fn random_when_nothing_chosen(
        &mut self,
        champion_id: ChampionId,
        catalog: &Catalog,
        finalization: bool,
        lcu_skin: Option<dekan_core::selection::SkinId>,
    ) {
        let fallback = RandomFallback {
            enabled: dekan_platform::preferences::RANDOM_SKIN.is_enabled(),
            finalization,
            target_chosen: self.state_rx.borrow().overlay_target.is_some()
                || self.custom_skin_selected(champion_id),
            already_rolled: self.random_rolled == Some(champion_id),
            declined: self.random_declined == Some(champion_id),
            lcu_skin,
        };
        if !should_roll_random(champion_id, fallback) {
            return;
        }
        self.random_rolled = Some(champion_id);
        info!(
            champion_id,
            "Champion locked with no skin chosen; rolling a random one so the match does not start without a skin"
        );
        self.roll_random(Some(catalog));
    }

    pub(super) fn roll_random(&self, catalog: Option<&Catalog>) {
        let Some(catalog) = catalog else {
            warn!("Random skin requested with no catalog loaded; ignoring it");
            return;
        };
        match catalog.roll_random(random_index) {
            Some(target) => {
                info!(
                    champion_id = target.champion_id,
                    skin_id = target.skin_id,
                    chroma_id = ?target.chroma_id,
                    "Random skin rolled as the injection target"
                );
                self.show_selection(
                    Some(target.package_entry_id()),
                    Some(SelectionOrigin::Random),
                );
                set_overlay_target(&self.state_tx, target);
            }
            None => info!(
                champion_id = catalog.champion_id,
                "Random skin requested, but the catalog has nothing besides the base skin"
            ),
        }
    }

    pub(super) fn show_selection(&self, entry_id: Option<u32>, origin: Option<SelectionOrigin>) {
        self.controller.set_selection(entry_id, origin);
    }
}
