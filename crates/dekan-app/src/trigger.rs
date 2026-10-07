use std::path::{Path, PathBuf};
use std::time::Duration;

use dekan_core::state::{InjectionStatus, StateReceiver, StateSender, set_injection_status};
use dekan_inject::overlay::OverlayConfig;
use dekan_inject::pipeline::{InjectionPipeline, PipelineConfig};
use dekan_platform::paths::{data_dir, state_dir, tools_dir_candidates};
use dekan_platform::process::ProcessFinder;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

mod mods;
mod paths;

pub use paths::{ResolvedPaths, ToolsSource, required_tool_files, tools_ready};

pub struct InjectionTrigger {
    state_tx: StateSender,
    state_rx: StateReceiver,
    paths: ResolvedPaths,
}

impl InjectionTrigger {
    pub fn new(state_tx: StateSender, state_rx: StateReceiver, paths: ResolvedPaths) -> Self {
        Self {
            state_tx,
            state_rx,
            paths,
        }
    }

    pub async fn run(&self, token: CancellationToken) {
        let mut state_rx = self.state_rx.clone();
        let mut injected_session = false;

        let mut active_overlay: Option<dekan_inject::overlay_process::OverlayProcess> = None;

        let mut armed: Option<ArmedPatcher> = None;

        let mut arming: Option<Arming<'_>> = None;
        let mut pending_arm: Option<ArmRequest> = None;
        let mut lcu_divergence_seen: Option<u32> = None;
        let mut tools_missing_reported = false;

        info!(
            tools = %self.paths.tools_dir.display(),
            library = %self.paths.library_dir.display(),
            game = %self.paths.game_dir.display(),
            "Injection supervisor initialized"
        );

        while !token.is_cancelled() {
            let arm_at = pending_arm.as_ref().map(|request| request.due);

            tokio::select! {
                _ = token.cancelled() => break,

                () = async {
                    match arm_at {
                        Some(due) => tokio::time::sleep_until(due).await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(request) = pending_arm.take() {

                        if !tools_ready(&self.paths) {
                            if !tools_missing_reported {
                                tools_missing_reported = true;
                                warn!(
                                    tools = %self.paths.tools_dir.display(),
                                    entry_id = ?request.key.entry_id,
                                    "Injection tools missing; this pick will not be injected and the client selection is left alone"
                                );
                            }
                            continue;
                        }

                        if let Some(superseded) = arming.take() {
                            info!(
                                superseded_entry = ?superseded.key.entry_id,
                                entry_id = ?request.key.entry_id,
                                "Selection changed while the patcher was being built; abandoning that build"
                            );
                        }

                        if let Some(previous) = armed.take() {
                            info!(
                                previous_entry = ?previous.key.entry_id,
                                entry_id = ?request.key.entry_id,
                                "Selection changed; restarting the patcher for the new skin"
                            );
                            previous.overlay.shutdown().await;
                        }
                        arming = Some(Arming {
                            key: request.key(),
                            task: Box::pin(self.arm_patcher(request)),
                        });
                    }
                }

                result = next_armed(&mut arming) => {
                    arming = None;
                    armed = result;
                    lcu_divergence_seen = None;
                }

                res = state_rx.changed() => {
                    if res.is_err() {
                        break;
                    }

                    let state = state_rx.borrow().clone();

                    if state.phase.is_in_game() && !injected_session {
                        injected_session = true;
                        pending_arm = None;

                        if let Some(in_flight) = arming.take() {

                            warn!(
                                champ_id = in_flight.key.champ_id,
                                entry_id = ?in_flight.key.entry_id,
                                "Game started before the patcher was armed; the skin will likely not load in this game process. Finishing the build for a reconnect"
                            );

                            let entry_id = in_flight.key.entry_id;
                            let mut phase_rx = state_rx.clone();
                            armed = tokio::select! {
                                _ = token.cancelled() => None,
                                result = in_flight.task => result,
                                () = match_ended(&mut phase_rx) => {
                                    info!(entry_id = ?entry_id, "Reconnect build abandoned: the match ended");
                                    continue;
                                }
                            };
                            if token.is_cancelled() {
                                break;
                            }
                        }

                        if armed.is_none() && !tools_ready(&self.paths) {
                            if !tools_missing_reported {
                                tools_missing_reported = true;
                                warn!(
                                    tools = %self.paths.tools_dir.display(),
                                    "Game started without the injection tools; nothing is injected this match"
                                );
                            }
                            continue;
                        }
                        info!(
                            phase = ?state.phase,
                            pre_armed = armed.is_some(),
                            armed_before_game = ?armed.as_ref().map(|a| a.armed_before_game),
                            "Game entered in-game phase; completing the injection"
                        );
                        active_overlay = self.handle_game_start(&state, armed.take(), &token).await;
                    } else if (state.phase.is_champ_select() || arms_in_lobby(&state)) && !injected_session {
                        let wanted = wanted_skin(&state);

                        let current = armed
                            .as_ref()
                            .map(|a| a.key)
                            .or_else(|| arming.as_ref().map(|a| a.key));

                        if armed.as_ref().is_some_and(|a| should_disarm(wanted, a.key)) {
                            if let Some(stale) = armed.take() {
                                info!(
                                    armed_entry = ?stale.key.entry_id,
                                    wanted = ?wanted,
                                    "Selection no longer matches the armed patcher; disarming it"
                                );
                                stale.overlay.shutdown().await;
                            }
                        }
                        if arming.as_ref().is_some_and(|a| should_disarm(wanted, a.key)) {
                            if let Some(stale) = arming.take() {
                                info!(
                                    building_entry = ?stale.key.entry_id,
                                    wanted = ?wanted,
                                    "Selection no longer matches the patcher being built; abandoning that build"
                                );
                            }
                        }

                        let divergence = armed.as_ref().filter(|a| !a.key.lobby).and_then(|a| {
                            let registered = a.lcu_skin?;
                            let live = state.selected_skin_id?;
                            (live != registered && lcu_divergence_seen != Some(live))
                                .then_some((a.key, registered, live))
                        });
                        if let Some((key, registered, live)) = divergence {
                            lcu_divergence_seen = Some(live);
                            info!(
                                registered,
                                live,
                                "Client skin selection moved away from the registered skin; registering it again"
                            );
                            let again = match key.entry_id {
                                Some(entry_id) => {
                                    self.register_in_champ_select(key.champ_id, entry_id).await
                                }
                                None => None,
                            };
                            if let Some(patcher) = armed.as_mut() {
                                patcher.lcu_skin = again;
                            }
                        }

                        match arm_decision(wanted, current) {
                            ArmDecision::Settled => pending_arm = None,
                            ArmDecision::Schedule(request) => {

                                let already_queued = pending_arm
                                    .as_ref()
                                    .is_some_and(|queued| queued.key() == request.key());
                                if !already_queued {
                                    debug!(
                                        champ_id = request.key.champ_id,
                                        entry_id = ?request.key.entry_id,
                                        mods_fingerprint = request.key.mods,
                                        classic_slot = ?request.key.classic_slot,
                                        "Overlay build scheduled for the chosen skin"
                                    );
                                    pending_arm = Some(request);
                                }
                            }
                        }
                    } else if state.phase.is_between_matches()
                        && (injected_session
                            || tools_missing_reported
                            || armed.is_some()
                            || arming.is_some()
                            || pending_arm.is_some())
                    {
                        debug!("Resetting injection session state for next match");
                        injected_session = false;
                        tools_missing_reported = false;
                        pending_arm = None;
                        lcu_divergence_seen = None;
                        if let Some(abandoned) = arming.take() {
                            info!(
                                entry_id = ?abandoned.key.entry_id,
                                phase = ?state.phase,
                                "Champ select ended before the patcher was built; abandoning the build"
                            );
                        }
                        if let Some(overlay) = active_overlay.take() {
                            overlay.shutdown().await;
                        }
                        if let Some(stale) = armed.take() {
                            stale.overlay.shutdown().await;
                        }
                        set_injection_status(&self.state_tx, InjectionStatus::Idle);
                    }
                }
            }
        }

        drop(arming);
        if let Some(stale) = armed.take() {
            stale.overlay.shutdown().await;
        }

        info!("Injection supervisor task terminated cleanly");
    }

    async fn arm_patcher(&self, request: ArmRequest) -> Option<ArmedPatcher> {
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

        let game_already_running = matches!(
            ProcessFinder::find_any_process(&dekan_platform::game_version::GAME_EXES),
            Ok(Some(_))
        );
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

    async fn build_and_arm(
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

    async fn reread_selection(
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

    async fn handle_game_start(
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

    async fn confirm_armed(
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

    async fn inject_late(
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

    async fn register_in_champ_select(&self, champ_id: u32, entry_id: u32) -> Option<u32> {
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

        let skin_id = dekan_lcu::skin_registration::skin_to_register(champ_id, entry_id, &owned);
        info!(
            champ_id,
            entry_id,
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

    async fn register_in_lobby(&self, picks: &[(u32, u32)]) {
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
        let skins: Vec<(u32, u32)> = picks
            .iter()
            .map(|&(champ_id, entry_id)| {
                (
                    champ_id,
                    dekan_lcu::skin_registration::skin_to_register(champ_id, entry_id, &owned),
                )
            })
            .collect();
        if let Err(e) = client.set_lobby_slot_skins(&skins).await {
            warn!(error = %e, skins = ?skins, "Could not register the skins in the lobby slots");
        }
    }

    async fn lcu_client(&self) -> Option<dekan_lcu::client::LcuClient> {
        dekan_lcu::client::LcuClient::discover()
            .await
            .inspect_err(|e| warn!(error = %e, "No LCU client; the skin cannot be registered in champ select"))
            .ok()
    }

    fn effective_game_dir(&self, pid: Option<u32>) -> PathBuf {
        let from_process = pid.and_then(|pid| match ProcessFinder::get_process_path(pid) {
            Ok(Some(exe_path)) => exe_path
                .parent()
                .and_then(dekan_platform::paths::normalize_game_dir),
            _ => None,
        });

        let dir = from_process
            .or_else(|| dekan_platform::paths::normalize_game_dir(&self.paths.game_dir))
            .or_else(dekan_platform::paths::discover_game_dir)
            .unwrap_or_else(|| self.paths.game_dir.clone());
        if dir.is_dir() {
            dekan_inject::overlay_builder::prewarm_game_index(&dir);
        }
        dir
    }

    fn pipeline_config(&self, game_dir: PathBuf) -> PipelineConfig {
        PipelineConfig {
            ltk_host_exe: self.paths.ltk_host_exe.clone(),
            ltk_dll_path: self.paths.ltk_dll_path.clone(),
            ltk_flags: dekan_inject::ltk_host::default_flags(),
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

    async fn release(armed: Option<ArmedPatcher>) {
        if let Some(armed) = armed {
            armed.overlay.shutdown().await;
        }
    }
}

const ARM_DEBOUNCE: Duration = Duration::from_millis(900);

const INITIAL_ARM_DEBOUNCE: Duration = Duration::from_millis(100);

fn prefer_lazy_wad_checks(game_dir: &Path) {
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
const GAME_PROCESS_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArmKey {
    champ_id: u32,

    entry_id: Option<u32>,

    mods: u64,

    classic_slot: Option<u32>,

    party: u64,

    second: Option<(u32, u32)>,

    lobby: bool,
}

impl ArmKey {
    fn covers(&self, champ_id: u32) -> bool {
        self.champ_id == champ_id || self.second.is_some_and(|(second, _)| second == champ_id)
    }

    fn entry_for(&self, champ_id: u32) -> Option<u32> {
        if champ_id == self.champ_id {
            self.entry_id
        } else {
            self.second
                .filter(|(second, _)| *second == champ_id)
                .map(|(_, entry_id)| entry_id)
        }
    }

    fn picks(&self) -> Vec<(u32, u32)> {
        self.entry_id
            .map(|entry_id| (self.champ_id, entry_id))
            .into_iter()
            .chain(self.second)
            .collect()
    }
}

fn lobby_mods_fingerprint(mods: &dekan_core::mods::ModSelection, champions: &[u32]) -> u64 {
    champions.iter().fold(0, |hash, champion| {
        hash.rotate_left(7) ^ mods.fingerprint(Some(*champion))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArmRequest {
    key: ArmKey,
    due: tokio::time::Instant,
}

impl ArmRequest {
    fn key(&self) -> ArmKey {
        self.key
    }
}

fn is_classic(champ_id: u32) -> bool {
    dekan_classic::builder::is_classic_champion(champ_id)
}

fn build_key(
    champ_id: u32,
    entry_id: Option<u32>,
    mods: &dekan_core::mods::ModSelection,
    client_skin: Option<u32>,
    party: u64,
) -> Option<ArmKey> {
    let classic = is_classic(champ_id);
    let entry_id = if classic {
        entry_id.filter(|e| dekan_classic::generator::skin_number(*e) != 0)
    } else {
        entry_id
    };
    let mods = if classic {
        0
    } else {
        mods.fingerprint(Some(champ_id))
    };
    let party = if classic { 0 } else { party };
    if entry_id.is_none() && mods == 0 && party == 0 {
        return None;
    }
    let classic_slot = if classic {
        client_skin
            .map(dekan_classic::generator::skin_number)
            .filter(|n| !dekan_classic::builder::CLASSIC_DEFAULT_SLOTS.contains(n))
    } else {
        None
    };
    Some(ArmKey {
        champ_id,
        entry_id,
        mods,
        classic_slot,
        party,
        second: None,
        lobby: false,
    })
}

fn arms_in_lobby(state: &dekan_core::state::AppState) -> bool {
    state.lobby.is_some() && state.phase.is_before_champ_select()
}

fn lobby_arm_key(state: &dekan_core::state::AppState) -> Option<ArmKey> {
    let lobby = state.lobby.as_ref()?;
    let mut chosen = lobby
        .chosen_in_slot_order()
        .into_iter()
        .map(|target| (target.champion_id, target.package_entry_id()))
        .filter(|&(champ_id, entry_id)| {
            !is_classic(champ_id) && !dekan_core::selection::is_base_skin(entry_id, champ_id)
        });
    let first = chosen.next();
    let second = chosen.next();
    let champions = lobby.champions();
    let champ_id = first
        .map(|(champ_id, _)| champ_id)
        .or_else(|| champions.first().copied())?;
    let mods = lobby_mods_fingerprint(&state.mods, &champions);
    if first.is_none() && mods == 0 {
        return None;
    }
    Some(ArmKey {
        champ_id,
        entry_id: first.map(|(_, entry)| entry),
        mods,
        classic_slot: None,
        party: 0,
        second,
        lobby: true,
    })
}

fn party_skins(state: &dekan_core::state::AppState) -> dekan_core::party::VerifiedParty {
    dekan_core::party::verified_party_skins(
        &state.party_peers,
        &state.team,
        state.local_puuid.as_deref(),
    )
}

struct ArmedPatcher {
    overlay: dekan_inject::overlay_process::OverlayProcess,
    key: ArmKey,

    armed_before_game: bool,

    lcu_skin: Option<u32>,
}

struct Arming<'a> {
    key: ArmKey,
    task: std::pin::Pin<Box<dyn std::future::Future<Output = Option<ArmedPatcher>> + Send + 'a>>,
}

async fn next_armed(arming: &mut Option<Arming<'_>>) -> Option<ArmedPatcher> {
    match arming {
        Some(in_flight) => in_flight.task.as_mut().await,
        None => std::future::pending().await,
    }
}

async fn match_ended(state_rx: &mut StateReceiver) {
    loop {
        if state_rx.borrow_and_update().phase.is_between_matches() {
            return;
        }
        if state_rx.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WantedSkin {
    Unknown,

    Nothing,

    Skin(ArmKey),
}

fn wanted_skin(state: &dekan_core::state::AppState) -> WantedSkin {
    if arms_in_lobby(state) {
        return match lobby_arm_key(state) {
            Some(key) => WantedSkin::Skin(key),
            None => WantedSkin::Nothing,
        };
    }
    let Some(champ_id) = state.champion_id else {
        let nothing_at_all = state.overlay_target.is_none() && state.mods.fingerprint(None) == 0;
        return if nothing_at_all {
            WantedSkin::Nothing
        } else {
            WantedSkin::Unknown
        };
    };

    let entry_id = state
        .overlay_target
        .as_ref()
        .filter(|target| target.matches_champion(champ_id))
        .map(dekan_core::overlay::OverlayTarget::package_entry_id)
        .filter(|entry_id| !dekan_core::selection::is_base_skin(*entry_id, champ_id));

    let (party, _) = party_skins(state);
    match build_key(
        champ_id,
        entry_id,
        &state.mods,
        state.selected_skin_id,
        dekan_core::party::party_fingerprint(&party),
    ) {
        Some(key) => WantedSkin::Skin(key),
        None => WantedSkin::Nothing,
    }
}

fn should_disarm(wanted: WantedSkin, key: ArmKey) -> bool {
    match wanted {
        WantedSkin::Unknown => false,
        WantedSkin::Nothing => true,
        WantedSkin::Skin(wanted) => wanted != key,
    }
}

enum ArmDecision {
    Settled,

    Schedule(ArmRequest),
}

fn arm_decision(wanted: WantedSkin, current: Option<ArmKey>) -> ArmDecision {
    let WantedSkin::Skin(key) = wanted else {
        return ArmDecision::Settled;
    };
    if current == Some(key) {
        return ArmDecision::Settled;
    }
    let debounce = if current.is_none() {
        INITIAL_ARM_DEBOUNCE
    } else {
        ARM_DEBOUNCE
    };
    ArmDecision::Schedule(ArmRequest {
        key,
        due: tokio::time::Instant::now() + debounce,
    })
}

#[cfg(test)]
#[path = "trigger_tests.rs"]
mod tests;
