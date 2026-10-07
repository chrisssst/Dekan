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
    state.phase.is_champ_select() || (state.lobby.is_some() && state.phase.is_before_champ_select())
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

type PendingImport = futures_util::future::BoxFuture<'static, Option<FinishedImport>>;

struct FinishedImport {
    category: ModCategory,
    champion: Option<ChampionId>,
    alias: Option<String>,
    source: PathBuf,
    outcome: Result<PathBuf, mods_store::ImportRefusal>,
    text: &'static dekan_platform::i18n::Text,
}

async fn next_import(pending: &mut Option<PendingImport>) -> Option<FinishedImport> {
    match pending {
        Some(task) => {
            let finished = task.await;
            *pending = None;
            finished
        }
        None => std::future::pending().await,
    }
}

async fn pick_and_import(
    owner: isize,
    own_root: PathBuf,
    category: ModCategory,
    champion: Option<ChampionId>,
    alias: Option<String>,
    text: &'static dekan_platform::i18n::Text,
) -> Option<FinishedImport> {
    let title = text.import_title;
    let picked = tokio::task::spawn_blocking(move || {
        dekan_platform::dialog::pick_file(
            owner,
            title,
            "Mods (*.fantome, *.zip, *.modpkg)",
            "*.fantome;*.zip;*.modpkg",
        )
    })
    .await;
    let source = match picked {
        Ok(Ok(Some(path))) => path,
        Ok(Ok(None)) => {
            debug!(category = ?category, "Mod import cancelled in the file dialog");
            return None;
        }
        Ok(Err(e)) => {
            warn!(error = %e, "The file dialog could not be shown; nothing imported");
            return None;
        }
        Err(e) => {
            error!(error = %e, "The file dialog task failed");
            return None;
        }
    };
    let from = source.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        mods_store::import_archive(&own_root, category, champion, &from)
    })
    .await;
    match outcome {
        Ok(outcome) => Some(FinishedImport {
            category,
            champion,
            alias,
            source,
            outcome,
            text,
        }),
        Err(e) => {
            error!(error = %e, "The mod import task failed");
            None
        }
    }
}

async fn next_preview(
    fetches: &mut Option<PreviewFetches>,
) -> Option<(u32, Result<std::sync::Arc<[u8]>, String>)> {
    match fetches {
        Some(stream) => futures_util::StreamExt::next(stream).await,
        None => std::future::pending().await,
    }
}

#[derive(Debug, Default)]
struct PreviewTally {
    asked: usize,
    fetched: usize,
    bytes: usize,
    failed: usize,
    first_error: Option<String>,
    started: Option<std::time::Instant>,
}

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

    preview_tally: PreviewTally,

    pending_import: Option<PendingImport>,
}

#[derive(Debug, Clone)]
pub struct ModsConfig {
    pub roots: Vec<ModRoot>,

    pub own_root: PathBuf,
    pub state_dir: PathBuf,

    pub game_dir: PathBuf,

    pub overlay_dir: PathBuf,

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
            preview_tally: PreviewTally::default(),
            pending_import: None,
        }
    }

    pub async fn run(mut self, token: CancellationToken) {
        let mut shown = false;
        let mut catalog_key = CatalogKey::default();
        let mut catalog_champion: Option<ChampionId> = None;
        let mut catalog: Option<Catalog> = None;

        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                command = self.commands.recv() => {
                    match command {
                        Some(OverlayCommand::SetMods { selection }) => {
                            self.apply_mod_request(&selection, catalog_champion);
                        }
                        Some(OverlayCommand::OpenModsFolder) => self.open_mods_folder(),
                        Some(OverlayCommand::Clear) => {
                            self.dismiss_historic();
                            self.random_declined = catalog_champion;
                            handle_command(&self.state_tx, OverlayCommand::Clear, catalog.as_ref());
                        }
                        Some(OverlayCommand::Random) => self.roll_random(catalog.as_ref()),
                        Some(OverlayCommand::ChromaPreview { id }) => {
                            self.send_chroma_preview(id, catalog.as_ref());
                        }
                        Some(OverlayCommand::TogglePreset) => self.toggle_preset(catalog_champion),
                        Some(OverlayCommand::SetProfile { name }) => {
                            if self.presets.switch_to(&name) {
                                info!(profile = %name, "Skin profile switched");
                                self.profile_changed(catalog_champion);
                            }
                        }
                        Some(OverlayCommand::NewProfile) => {
                            let locale = catalog.as_ref().and_then(|c| c.locale.clone());
                            self.new_profile(locale.as_deref(), catalog_champion);
                        }
                        Some(OverlayCommand::DeleteProfile) => {
                            if let Some(name) = self.presets.delete_active() {
                                info!(profile = %name, "Skin profile deleted; the default profile is active");
                                self.profile_changed(catalog_champion);
                            }
                        }
                        Some(OverlayCommand::FocusChampion { id }) => {
                            if focus_lobby_champion(&self.state_tx, id) {
                                info!(champion_id = id, "Lobby champion shown in the overlay");
                            }
                        }
                        Some(OverlayCommand::ImportMod { category }) => {
                            let locale = catalog.as_ref().and_then(|c| c.locale.clone());
                            let alias = catalog.as_ref().and_then(|c| c.alias.clone());
                            self.start_import(category, catalog_champion, alias, locale.as_deref());
                        }
                        Some(command) => handle_command(&self.state_tx, command, catalog.as_ref()),

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
                () = tokio::time::sleep(TRACK_INTERVAL) => {
                    let (wanted, finalization, champion, target_stale, target, lcu_skin, confirmed, lobby) = {
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
                        )
                    };
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
                        if champion != catalog_champion {
                            self.chroma_previews.clear();
                        }
                        catalog_champion = champion;
                        catalog = self.refresh_catalog(champion, key.lobby.as_deref()).await;
                        self.preview_fetches = None;
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
        let chromas = built
            .skins
            .iter()
            .map(|skin| skin.chromas.len())
            .sum::<usize>();
        let without_preview: Vec<u32> = built
            .skins
            .iter()
            .flat_map(|skin| skin.chromas.iter())
            .filter(|chroma| !chroma.has_preview)
            .map(|chroma| chroma.id)
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
            self.warm_companions(built.alias.clone());
        }
        Some(built)
    }
}

impl OverlaySession {
    fn warm_companions(&self, alias: Option<String>) {
        let Some(alias) = alias else {
            return;
        };
        let Some(game_dir) = dekan_platform::paths::normalize_game_dir(&self.mods.game_dir)
            .or_else(dekan_platform::paths::discover_game_dir)
        else {
            return;
        };
        let cache_dir = self.mods.state_dir.clone();
        let overlay_dir = self.mods.overlay_dir.clone();
        let state_rx = self.state_rx.clone();
        drop(tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let companions =
                match dekan_classic::generator::StandardChampion::open(&game_dir, &alias)
                    .map(|champion| champion.with_cache_dir(&cache_dir).companions())
                {
                    Ok(companions) => companions,
                    Err(e) => {
                        debug!(error = %e, "Companion characters not indexed ahead of the build");
                        return;
                    }
                };
            let skin_bins: Vec<u64> = std::iter::once(alias.to_ascii_lowercase())
                .chain(companions.iter().cloned())
                .map(|character| {
                    dekan_wad::hash::wad_path_hash(&format!(
                        "data/characters/{character}/skins/skin0.bin"
                    ))
                })
                .collect();
            match dekan_inject::overlay_builder::prewarm_shared_copies(
                &game_dir,
                &overlay_dir,
                &skin_bins,
                &|| {
                    state_rx.borrow().phase.is_in_game()
                        && !dekan_inject::overlay_builder::build_waiting_for_copies()
                },
            ) {
                Ok(copied) => info!(
                    companions = ?companions,
                    copied,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Shared map WADs prepared in the background; the selection window stayed responsive"
                ),
                Err(dekan_inject::error::InjectError::Cancelled) => info!(
                    elapsed_ms = started.elapsed().as_millis(),
                    "Map WAD copy ahead stopped: the match started and no build needs it"
                ),
                Err(e) => {
                    warn!(error = %e, "Map WAD not copied ahead; the first build of this skin copies it")
                }
            }
        }));
    }

    async fn refresh_mods(&mut self, champion_id: ChampionId, alias: Option<String>) -> ModsPanel {
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

    fn apply_mod_request(
        &self,
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

        self.controller.set_mod_selection(next.view(champion));
    }

    fn track_historic(
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
        if !historic::may_restore(champion_id, target.is_some(), lcu_skin) {
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

    fn saved_skin(
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

    fn restore_lobby_champions(&mut self, lobby: &[ChampionId], focused: Option<ChampionId>) {
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
            if !historic::may_restore(champion_id, chosen, slot_skin) {
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

    fn publish_presets(&self, champion: Option<ChampionId>) {
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

    fn toggle_preset(&mut self, champion: Option<ChampionId>) {
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

    fn new_profile(&mut self, locale: Option<&str>, champion: Option<ChampionId>) {
        let base = dekan_platform::i18n::Language::for_locale(locale)
            .text()
            .overlay_profile_base;
        let name = self.presets.create(base);
        info!(profile = %name, "Skin profile created and switched to");
        self.profile_changed(champion);
    }

    fn profile_changed(&mut self, champion: Option<ChampionId>) {
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

    fn record_historic(&mut self, confirmed: bool, target: Option<&OverlayTarget>) {
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

    fn dismiss_historic(&mut self) {
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

    fn random_when_nothing_chosen(
        &mut self,
        champion_id: ChampionId,
        catalog: &Catalog,
        finalization: bool,
        lcu_skin: Option<dekan_core::selection::SkinId>,
    ) {
        let fallback = RandomFallback {
            enabled: dekan_platform::preferences::RANDOM_SKIN.is_enabled(),
            finalization,
            target_chosen: self.state_rx.borrow().overlay_target.is_some(),
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

    fn roll_random(&self, catalog: Option<&Catalog>) {
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

    fn send_chroma_preview(&mut self, chroma_id: u32, catalog: Option<&Catalog>) {
        if let Some(image) = self.chroma_previews.get(&chroma_id) {
            debug!(
                chroma_id,
                bytes = image.len(),
                "Chroma preview served from the session cache"
            );
            self.controller.set_chroma_preview(chroma_id, image.clone());
            return;
        }
        if self.preview_fetches.is_some() {
            debug!(
                chroma_id,
                asked = self.preview_tally.asked,
                fetched = self.preview_tally.fetched,
                failed = self.preview_tally.failed,
                "Chroma preview asked while previews are still being fetched; it is shown when it arrives"
            );
            return;
        }
        let Some(path) = catalog.and_then(|c| c.chroma_preview_path(chroma_id)) else {
            debug!(chroma_id, "Chroma preview asked for an entry without one");
            return;
        };
        debug!(chroma_id, path, "Chroma preview fetched again on hover");
        self.start_preview_fetches(vec![(chroma_id, path.to_owned())]);
    }

    fn start_preview_fetches(&mut self, previews: Vec<(u32, String)>) {
        if previews.is_empty() {
            return;
        }
        self.preview_tally = PreviewTally {
            asked: previews.len(),
            started: Some(std::time::Instant::now()),
            ..PreviewTally::default()
        };
        self.preview_fetches = Some(catalog::chroma_preview_fetches(previews));
    }

    fn deliver_chroma_preview(&mut self, chroma_id: u32, image: std::sync::Arc<[u8]>) {
        self.preview_tally.fetched += 1;
        self.preview_tally.bytes += image.len();
        debug!(
            chroma_id,
            bytes = image.len(),
            "Chroma preview fetched from the client"
        );
        self.controller.set_chroma_preview(chroma_id, image.clone());
        self.chroma_previews.insert(chroma_id, image);
    }

    fn count_failed_preview(&mut self, chroma_id: u32, reason: String) {
        debug!(chroma_id, reason, "Chroma preview not fetched");
        self.preview_tally.failed += 1;
        self.preview_tally.first_error.get_or_insert(reason);
    }

    fn finish_preview_fetches(&mut self) {
        self.preview_fetches = None;
        let tally = std::mem::take(&mut self.preview_tally);
        let elapsed_ms = tally
            .started
            .map_or(0, |started| started.elapsed().as_millis());
        if tally.asked <= 1 {
            debug!(
                fetched = tally.fetched,
                elapsed_ms,
                error = tally.first_error.as_deref().unwrap_or("-"),
                "Chroma preview fetch on hover finished"
            );
        } else if tally.failed == 0 {
            info!(
                asked = tally.asked,
                fetched = tally.fetched,
                bytes = tally.bytes,
                elapsed_ms,
                "Chroma previews fetched from the client"
            );
        } else {
            warn!(
                asked = tally.asked,
                fetched = tally.fetched,
                failed = tally.failed,
                elapsed_ms,
                first_error = tally.first_error.as_deref().unwrap_or("-"),
                "Some chroma previews could not be fetched from the client; their hover shows no image"
            );
        }
    }

    fn show_selection(&self, entry_id: Option<u32>, origin: Option<SelectionOrigin>) {
        self.controller.set_selection(entry_id, origin);
    }

    fn start_import(
        &mut self,
        category: ModCategory,
        champion: Option<ChampionId>,
        alias: Option<String>,
        locale: Option<&str>,
    ) {
        if self.pending_import.is_some() {
            debug!(category = ?category, "A mod import is already open; this request is ignored");
            return;
        }
        let text = dekan_platform::i18n::Language::for_locale(locale).text();
        self.pending_import = Some(Box::pin(pick_and_import(
            self.controller.window_handle(),
            self.mods.own_root.clone(),
            category,
            champion,
            alias,
            text,
        )));
    }

    async fn finish_import(&mut self, finished: FinishedImport) {
        let FinishedImport {
            category,
            champion,
            alias,
            source,
            outcome,
            text,
        } = finished;
        match outcome {
            Ok(destination) => {
                info!(
                    category = ?category,
                    source = %source.display(),
                    destination = %destination.display(),
                    "Custom mod imported"
                );
                if let Some(champion_id) = champion {
                    let panel = self.refresh_mods(champion_id, alias).await;
                    self.controller.set_mods(panel);
                }
            }
            Err(reason) => {
                warn!(
                    category = ?category,
                    source = %source.display(),
                    reason = %reason,
                    "Custom mod import refused"
                );
                let message = dekan_platform::i18n::fill(
                    text.import_refused,
                    "reason",
                    &reason.describe(text),
                );
                let title = text.import_title;
                drop(tokio::task::spawn_blocking(move || {
                    dekan_platform::shell::message_box(title, &message);
                }));
            }
        }
    }

    fn open_mods_folder(&self) {
        match dekan_platform::shell::open_folder(&self.mods.own_root) {
            Ok(()) => info!(folder = %self.mods.own_root.display(), "Custom mods folder opened"),
            Err(e) => warn!(error = %e, "Could not open the custom mods folder"),
        }
    }
}

fn random_index(bound: usize) -> usize {
    use std::hash::{BuildHasher, Hasher};
    let value = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    (value % bound.max(1) as u64) as usize
}

fn handle_command(state_tx: &StateSender, command: OverlayCommand, catalog: Option<&Catalog>) {
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

#[cfg(test)]
#[path = "overlay_session_tests.rs"]
mod tests;
