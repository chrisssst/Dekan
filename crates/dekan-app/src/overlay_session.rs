use std::path::PathBuf;

use dekan_core::historic::{self, HistoricBook};
use dekan_core::mods::{ModCatalog, ModCategory, ModRoot};
use dekan_core::overlay::{OverlayCommand, OverlayTarget};
use dekan_core::phase::GamePhase;
use dekan_core::selection::ChampionId;
use dekan_core::state::{
    InjectionStatus, StateReceiver, StateSender, clear_overlay_target, set_mod_selection,
    set_overlay_target,
};
use dekan_platform::overlay_window::{OverlayController, track_once};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::catalog::{self, Catalog, ModsPanel, PreviewFetches};
use crate::{historic_store, mods_store};

const TRACK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

const EMPTY_CATALOG_JSON: &str = r#"{"championId":0,"championName":"","skins":[]}"#;

#[must_use]
fn target_is_stale(target: Option<&OverlayTarget>, champion: Option<ChampionId>) -> bool {
    match (target, champion) {
        (Some(target), Some(champion)) => !target.matches_champion(champion),
        _ => false,
    }
}

#[must_use]
fn wanted_for_phase(phase: &GamePhase) -> bool {
    matches!(phase, GamePhase::ChampSelect | GamePhase::Finalization)
}

async fn next_preview(fetches: &mut Option<PreviewFetches>) -> Option<(u32, Option<String>)> {
    match fetches {
        Some(stream) => futures_util::StreamExt::next(stream).await,
        None => std::future::pending().await,
    }
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
        && fallback.lcu_skin.is_none_or(|skin| {
            dekan_core::selection::SelectionMode::is_base_skin(skin, champion_id)
        })
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

    historic_restored: Option<OverlayTarget>,

    historic_consulted: Option<ChampionId>,

    historic_recorded: Option<OverlayTarget>,

    random_rolled: Option<ChampionId>,

    random_declined: Option<ChampionId>,

    chroma_previews: std::collections::HashMap<u32, String>,

    preview_fetches: Option<PreviewFetches>,
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
        Self {
            controller,
            commands,
            state_tx,
            state_rx,
            library_root,
            mods,
            mod_catalog: ModCatalog::default(),
            historic,
            historic_restored: None,
            historic_consulted: None,
            historic_recorded: None,
            random_rolled: None,
            random_declined: None,
            chroma_previews: std::collections::HashMap::new(),
            preview_fetches: None,
        }
    }

    pub async fn run(mut self, token: CancellationToken) {
        let mut shown = false;
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
                        Some(OverlayCommand::ImportMod { category }) => {
                            let locale = catalog.as_ref().and_then(|c| c.locale.clone());
                            let alias = catalog.as_ref().and_then(|c| c.alias.clone());
                            self.import_mod(category, catalog_champion, alias, locale.as_deref())
                                .await;
                        }
                        Some(command) => handle_command(&self.state_tx, command, catalog.as_ref()),

                        None => {
                            warn!("Overlay command channel closed; the selection UI is gone");
                            break;
                        }
                    }
                }
                fetched = next_preview(&mut self.preview_fetches) => match fetched {
                    Some((id, Some(uri))) => self.deliver_chroma_preview(id, uri),
                    Some((id, None)) => debug!(chroma_id = id, "Chroma preview not fetched"),
                    None => self.preview_fetches = None,
                },
                () = tokio::time::sleep(TRACK_INTERVAL) => {
                    let (wanted, finalization, champion, target_stale, target, lcu_skin, confirmed) = {
                        let state = self.state_rx.borrow_and_update();
                        (
                            wanted_for_phase(&state.phase),
                            state.phase == GamePhase::Finalization,
                            state.champion_id,
                            target_is_stale(state.overlay_target.as_ref(), state.champion_id),
                            state.overlay_target.clone(),
                            state.selected_skin_id,
                            state.injection == InjectionStatus::Confirmed,
                        )
                    };
                    self.record_historic(confirmed, target.as_ref());

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
                    if champion != catalog_champion {
                        catalog_champion = champion;
                        self.chroma_previews.clear();
                        catalog = self.refresh_catalog(champion).await;
                        self.preview_fetches = catalog
                            .as_ref()
                            .map(|built| catalog::chroma_preview_fetches(built.chroma_preview_paths()));
                        if champion.is_none() {
                            self.historic_consulted = None;
                            self.historic_restored = None;
                            self.random_rolled = None;
                            self.random_declined = None;
                        }
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

    async fn refresh_catalog(&mut self, champion: Option<ChampionId>) -> Option<Catalog> {
        let Some(champion_id) = champion else {
            self.controller.set_catalog(EMPTY_CATALOG_JSON.to_owned());
            return None;
        };

        let classic = dekan_classic::builder::ClassicIdMapper::is_classic_champion(champion_id);
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
        if !self.mods.injection_tools.iter().all(|file| file.is_file()) {
            built.notice = Some(catalog::CatalogNotice::ToolsMissing);
        }
        match catalog::catalog_json(&built) {
            Ok(json) => {
                info!(
                    champion_id,
                    champion = %built.champion_name,
                    skins = built.skins.len(),
                    entries = built.entry_count(),
                    custom_mods = built.mods.available.len(),
                    "Skin catalog sent to the overlay"
                );
                self.controller.set_catalog(json);
                if !classic {
                    self.warm_companions(built.alias.clone());
                }
                Some(built)
            }
            Err(e) => {
                error!(error = %e, champion_id, "Could not serialize the skin catalog");
                None
            }
        }
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
            ) {
                Ok(copied) => info!(
                    companions = ?companions,
                    copied,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Shared map WADs prepared in the background; the selection window stayed responsive"
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

        match serde_json::to_string(&next.view(champion)) {
            Ok(json) => self
                .controller
                .eval_script(format!("window.dekanOverlay.setModSelection({json});")),
            Err(e) => error!(error = %e, "Could not serialize the mod selection for the overlay"),
        }
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

        let Some(entry) = self.historic.get(champion_id) else {
            return;
        };
        if !historic::may_restore(champion_id, target.is_some(), lcu_skin) {
            debug!(
                champion_id,
                lcu_skin = ?lcu_skin,
                chosen = target.is_some(),
                "Historic skin not restored: a choice is already made"
            );
            return;
        }

        let Some(restored) = catalog.resolve_target(entry.package_entry_id()) else {
            info!(
                champion_id,
                entry_id = entry.package_entry_id(),
                "Historic skin is no longer in the library; not restored"
            );
            return;
        };
        info!(
            champion_id,
            skin_id = restored.skin_id,
            chroma_id = ?restored.chroma_id,
            "Historic skin restored as the injection target"
        );
        set_overlay_target(&self.state_tx, restored.clone());
        self.show_selection(Some(restored.package_entry_id()), Some("historic"));
        self.historic_restored = Some(restored);
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
                self.show_selection(Some(target.package_entry_id()), Some("random"));
                set_overlay_target(&self.state_tx, target);
            }
            None => info!(
                champion_id = catalog.champion_id,
                "Random skin requested, but the catalog has nothing besides the base skin"
            ),
        }
    }

    fn send_chroma_preview(&mut self, chroma_id: u32, catalog: Option<&Catalog>) {
        if let Some(uri) = self.chroma_previews.get(&chroma_id) {
            self.push_chroma_preview(chroma_id, uri);
            return;
        }
        if self.preview_fetches.is_some() {
            return;
        }
        let Some(path) = catalog.and_then(|c| c.chroma_preview_path(chroma_id)) else {
            debug!(chroma_id, "Chroma preview asked for an entry without one");
            return;
        };
        self.preview_fetches = Some(catalog::chroma_preview_fetches(vec![(
            chroma_id,
            path.to_owned(),
        )]));
    }

    fn deliver_chroma_preview(&mut self, chroma_id: u32, uri: String) {
        self.push_chroma_preview(chroma_id, &uri);
        self.chroma_previews.insert(chroma_id, uri);
    }

    fn push_chroma_preview(&self, chroma_id: u32, uri: &str) {
        match serde_json::to_string(uri) {
            Ok(json) => self.controller.eval_script(format!(
                "window.dekanOverlay.setChromaPreview({chroma_id}, {json});"
            )),
            Err(e) => warn!(error = %e, chroma_id, "Could not serialize a chroma preview"),
        }
    }

    fn show_selection(&self, entry_id: Option<u32>, origin: Option<&str>) {
        let id = entry_id.map_or_else(|| "null".to_owned(), |id| id.to_string());
        let origin = origin.map_or_else(|| "null".to_owned(), |o| format!("\"{o}\""));
        self.controller
            .eval_script(format!("window.dekanOverlay.setSelection({id}, {origin});"));
    }

    async fn import_mod(
        &mut self,
        category: ModCategory,
        champion: Option<ChampionId>,
        alias: Option<String>,
        locale: Option<&str>,
    ) {
        let text = dekan_platform::i18n::Language::for_locale(locale).text();
        let title = text.import_title;
        let owner = self.controller.window_handle();
        let picked = tokio::task::spawn_blocking(move || {
            dekan_platform::dialog::pick_file(
                owner,
                title,
                "Mods (*.fantome, *.zip)",
                "*.fantome;*.zip",
            )
        })
        .await;
        let source = match picked {
            Ok(Ok(Some(path))) => path,
            Ok(Ok(None)) => {
                debug!(category = ?category, "Mod import cancelled in the file dialog");
                return;
            }
            Ok(Err(e)) => {
                warn!(error = %e, "The file dialog could not be shown; nothing imported");
                return;
            }
            Err(e) => {
                error!(error = %e, "The file dialog task failed");
                return;
            }
        };

        let own_root = self.mods.own_root.clone();
        let from = source.clone();
        let imported = tokio::task::spawn_blocking(move || {
            mods_store::import_archive(&own_root, category, champion, &from)
        })
        .await;
        match imported {
            Ok(Ok(destination)) => {
                info!(
                    category = ?category,
                    source = %source.display(),
                    destination = %destination.display(),
                    "Custom mod imported"
                );
                if let Some(champion_id) = champion {
                    let panel = self.refresh_mods(champion_id, alias).await;
                    match serde_json::to_string(&panel) {
                        Ok(json) => self
                            .controller
                            .eval_script(format!("window.dekanOverlay.setMods({json});")),
                        Err(e) => error!(error = %e, "Could not serialize the mods panel"),
                    }
                }
            }
            Ok(Err(reason)) => {
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

                // ignore-ok: the refusal is already logged; the box only tells the user why
                let _ = tokio::task::spawn_blocking(move || {
                    dekan_platform::shell::message_box(title, &message);
                })
                .await;
            }
            Err(e) => error!(error = %e, "The mod import task failed"),
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
        | OverlayCommand::Random => {
            warn!("Session command reached the skin handler; ignoring it");
        }
    }
}

#[cfg(test)]
#[path = "overlay_session_tests.rs"]
mod tests;
