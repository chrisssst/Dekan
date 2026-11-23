use std::path::PathBuf;

use dekan_core::historic::{self, HistoricBook};
use dekan_core::mods::{ModCatalog, ModCategory, ModRoot};
use dekan_core::overlay::{OverlayCommand, OverlayTarget, PresetsView, SelectionOrigin};
use dekan_core::phase::GamePhase;
use dekan_core::presets::PresetBook;
use dekan_core::selection::ChampionId;
use dekan_core::state::{
    AppState, InjectionStatus, StateReceiver, StateSender, clear_lobby_target,
    clear_overlay_target, focus_lobby_champion, set_lobby_target, set_mod_selection,
    set_overlay_target,
};
use dekan_platform::overlay_window::{OverlayController, track_once};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::catalog::{self, Catalog, ModsPanel, PreviewFetches};
use crate::{historic_store, mods_store, preset_store};

const TRACK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

#[must_use]
fn target_is_stale(target: Option<&OverlayTarget>, champion: Option<ChampionId>) -> bool {
    match (target, champion) {
        (Some(target), Some(champion)) => !target.matches_champion(champion),
        _ => false,
    }
}

#[must_use]
fn wanted_for_state(state: &AppState) -> bool {
    state.phase.is_champ_select()
        || (state.lobby.is_some() && state.phase.is_before_champ_select())
        || (state.phase == GamePhase::Reconnect
            && dekan_platform::preferences::INJECTION.is_enabled())
}

#[must_use]
fn last_call_for_a_skin(state: &AppState) -> bool {
    state.phase == GamePhase::Finalization
        || (state.lobby.is_some()
            && matches!(state.phase, GamePhase::Matchmaking | GamePhase::ReadyCheck))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CatalogKey {
    champion: Option<ChampionId>,
    lobby: Option<Vec<ChampionId>>,
}

pub struct OverlaySession {
    controller: OverlayController,
    commands: UnboundedReceiver<OverlayCommand>,
    state_tx: StateSender,
    state_rx: StateReceiver,
    library_root: PathBuf,
    mods: ModsConfig,

    mod_catalog: ModCatalog,

    historic: HistoricBook,

    presets: PresetBook,

    historic_restored: Option<OverlayTarget>,

    restored_from_preset: bool,

    lobby_restored: std::collections::HashSet<ChampionId>,

    lobby_auto: std::collections::HashSet<ChampionId>,

    declined_presets: std::collections::HashSet<ChampionId>,

    historic_consulted: Option<ChampionId>,

    historic_recorded: Option<OverlayTarget>,

    random_rolled: Option<ChampionId>,

    random_declined: Option<ChampionId>,

    chroma_previews: std::collections::HashMap<u32, std::sync::Arc<[u8]>>,

    preview_fetches: Option<PreviewFetches>,

    urgent_preview: Option<catalog::UrgentPreview>,

    preview_tally: PreviewTally,

    pending_import: Option<PendingImport>,

    warm_turn: std::sync::Arc<std::sync::atomic::AtomicU64>,

    panel_links: Option<dekan_platform::panel::PanelLinks>,
}

#[derive(Debug, Clone)]
pub struct ModsConfig {
    pub roots: Vec<ModRoot>,

    pub own_root: PathBuf,
    pub state_dir: PathBuf,

    pub game_dir: PathBuf,

    pub injection_tools: Vec<PathBuf>,
}

impl OverlaySession {
    pub fn new(
        controller: OverlayController,
        commands: UnboundedReceiver<OverlayCommand>,
        state_tx: StateSender,
        state_rx: StateReceiver,
        library_root: PathBuf,
        mods: ModsConfig,
    ) -> Self {
        let historic = historic_store::load(&mods.state_dir);
        let presets = preset_store::load(&mods.state_dir);
        if let Some(identity) = mods::load_identity(&mods.state_dir) {
            dekan_core::state::set_local_identity(&state_tx, Some(identity));
        }
        Self {
            controller,
            commands,
            state_tx,
            state_rx,
            library_root,
            mods,
            mod_catalog: ModCatalog::default(),
            historic,
            presets,
            historic_restored: None,
            restored_from_preset: false,
            lobby_restored: std::collections::HashSet::new(),
            lobby_auto: std::collections::HashSet::new(),
            declined_presets: std::collections::HashSet::new(),
            historic_consulted: None,
            historic_recorded: None,
            random_rolled: None,
            random_declined: None,
            chroma_previews: std::collections::HashMap::new(),
            preview_fetches: None,
            urgent_preview: None,
            preview_tally: PreviewTally::default(),
            pending_import: None,
            warm_turn: std::sync::Arc::default(),
            panel_links: None,
        }
    }

    #[must_use]
    pub fn with_panel_links(mut self, links: Option<dekan_platform::panel::PanelLinks>) -> Self {
        self.panel_links = links;
        self
    }

    pub async fn run(mut self, token: CancellationToken) {
        let mut shown = false;
        let mut catalog_key = CatalogKey::default();
        let mut catalog_champion: Option<ChampionId> = None;
        let mut catalog: Option<Catalog> = None;
        let mut party_shown = None;

        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                command = self.commands.recv() => {
                    match command {
                        Some(cmd) => self.dispatch_command(cmd, catalog_champion, catalog.as_ref()),
                        None => {
                            warn!("Overlay command channel closed; the selection UI is gone");
                            break;
                        }
                    }
                }
                finished = next_import(&mut self.pending_import) => {
                    if let Some(finished) = finished {
                        self.finish_import(finished).await;
                    }
                }
                fetched = next_preview(&mut self.preview_fetches) => match fetched {
                    Some((id, Ok(image))) => self.deliver_chroma_preview(id, image),
                    Some((id, Err(reason))) => self.count_failed_preview(id, reason),
                    None => self.finish_preview_fetches(),
                },
                (id, outcome) = next_urgent_preview(&mut self.urgent_preview) => match outcome {
                    Ok(image) => self.deliver_chroma_preview(id, image),
                    Err(reason) => debug!(chroma_id = id, reason, "Urgent chroma preview on hover not fetched"),
                },
                () = tokio::time::sleep(TRACK_INTERVAL) => {
                    let (wanted, finalization, mut champion, target_stale, target, lcu_skin, confirmed, lobby, phase) = {
                        let state = self.state_rx.borrow_and_update();
                        (
                            wanted_for_state(&state),
                            last_call_for_a_skin(&state),
                            state.champion_id,
                            target_is_stale(state.overlay_target.as_ref(), state.champion_id),
                            state.overlay_target.clone(),
                            state.selected_skin_id,
                            state.injection == InjectionStatus::Confirmed,
                            state.lobby.as_ref().map(dekan_core::lobby::LobbyPicks::champions),
                            state.phase,
                        )
                    };
                    if phase == GamePhase::Reconnect && champion.is_none() {
                        champion = self.recover_reconnect_champion().await;
                    }
                    let party = {
                        let state = self.state_rx.borrow();
                        dekan_core::party::party_summary(
                            &state.shown_party_status(),
                            &state.party_peers,
                            &state.team,
                            state.local_puuid.as_deref(),
                        )
                    };
                    if party != party_shown {
                        party_shown = party;
                        self.controller.set_party(party);
                    }
                    self.record_historic(confirmed, target.as_ref());
                    if lobby.is_none() {
                        self.lobby_restored.clear();
                        self.lobby_auto.clear();
                    }

                    let placement = track_once(&self.controller, wanted);
                    let now_shown = placement.is_some();
                    if now_shown != shown {
                        shown = now_shown;
                        info!(shown = shown, "Overlay visibility changed");
                    }

                    if target_stale {
                        warn!(
                            champion_id = ?champion,
                            "Champion changed under the chosen skin; dropping the target"
                        );
                        clear_overlay_target(&self.state_tx);
                    }

                    let champion = wanted.then_some(champion).flatten();
                    let key = CatalogKey {
                        champion,
                        lobby: wanted.then(|| lobby.clone()).flatten(),
                    };
                    if key != catalog_key {
                        catalog_key = key.clone();
                        catalog_champion = champion;
                        catalog = self.refresh_catalog(champion, key.lobby.as_deref()).await;
                        self.preview_fetches = None;
                        self.urgent_preview = None;
                        if let Some(built) = catalog.as_ref() {
                            self.start_preview_fetches(built.chroma_preview_paths());
                        }
                        self.publish_presets(champion);
                        if let Some(current) = target
                            .as_ref()
                            .filter(|t| !target_stale && Some(t.champion_id) == champion)
                        {
                            self.show_selection(Some(current.package_entry_id()), None);
                        }
                        if champion.is_none() {
                            self.declined_presets.clear();
                            self.historic_consulted = None;
                            self.historic_restored = None;
                            self.random_rolled = None;
                            self.random_declined = None;
                        }
                    }
                    if let Some(lobby) = wanted.then_some(lobby.as_deref()).flatten() {
                        self.restore_lobby_champions(lobby, champion);
                    }
                    if let (Some(champion_id), Some(built)) = (champion, catalog.as_ref()) {
                        let target = if target_stale { None } else { target };
                        self.track_historic(champion_id, built, target.as_ref(), lcu_skin);
                        self.random_when_nothing_chosen(champion_id, built, finalization, lcu_skin);
                    }

                }
            }
        }

        self.controller.hide();
        info!("Overlay session terminated");
    }

    async fn refresh_catalog(
        &mut self,
        champion: Option<ChampionId>,
        lobby: Option<&[ChampionId]>,
    ) -> Option<Catalog> {
        let Some(champion_id) = champion else {
            self.controller.set_catalog(Catalog {
                notice: lobby.map(|_| catalog::CatalogNotice::LobbyWaiting),
                ..Catalog::default()
            });
            return None;
        };

        let classic = dekan_classic::builder::is_classic_champion(champion_id);
        let mut built = if classic {
            let game_dir = dekan_platform::paths::normalize_game_dir(&self.mods.game_dir)
                .or_else(dekan_platform::paths::discover_game_dir)
                .unwrap_or_else(|| self.mods.game_dir.clone());
            catalog::load_classic_catalog(game_dir, self.library_root.clone(), champion_id).await
        } else {
            catalog::load_catalog(self.library_root.clone(), champion_id).await
        };
        if !classic {
            built.mods = self.refresh_mods(champion_id, built.alias.clone()).await;
        }
        if let Some(lobby) = lobby {
            built.lobby = catalog::lobby_champions(lobby).await;
            built.notice = Some(catalog::CatalogNotice::LobbyChampions);
        }
        if !self.mods.injection_tools.iter().all(|file| file.is_file()) {
            built.notice = Some(catalog::CatalogNotice::ToolsMissing);
        }
        let chromas: usize = built.skins.iter().map(|s| s.chromas.len()).sum();
        let without_preview: Vec<u32> = built
            .skins
            .iter()
            .flat_map(|s| s.chromas.iter().filter(|c| !c.has_preview).map(|c| c.id))
            .collect();
        info!(
            champion_id,
            champion = %built.champion_name,
            skins = built.skins.len(),
            entries = built.entry_count(),
            chromas,
            chromas_with_preview = chromas - without_preview.len(),
            custom_mods = built.mods.available.len(),
            "Skin catalog sent to the overlay"
        );
        if !without_preview.is_empty() {
            debug!(champion_id, ids = ?without_preview, "Chromas the client gave no preview image for");
        }
        self.controller.set_catalog(built.clone());
        if !classic {
            self.warm_champion(built.alias.clone());
        }
        Some(built)
    }
}

mod choice;

use choice::handle_command;
pub use choice::{RandomFallback, should_roll_random};
use import::{PendingImport, next_import};
use preview::{PreviewTally, next_preview, next_urgent_preview};
mod import;
mod mods;
mod preview;

#[cfg(test)]
mod tests;
