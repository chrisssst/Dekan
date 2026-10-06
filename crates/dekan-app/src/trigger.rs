use std::path::{Path, PathBuf};
use std::time::Duration;

use dekan_core::state::{InjectionStatus, StateReceiver, StateSender, set_injection_status};
use dekan_inject::overlay::OverlayConfig;
use dekan_inject::pipeline::{InjectionPipeline, PipelineConfig};
use dekan_platform::paths::{data_dir, state_dir, tools_dir_candidates};
use dekan_platform::process::ProcessFinder;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

pub const AUDITED_LTK_DLL_HASH: &str =
    "07a43bf36a389eb00f6276e333bd7f2b95218f25a58e1e128ff4d2e4ab2dc99b";

pub const AUDITED_LTK_HOST_HASH: &str =
    "a7c4047ce7548c7ae820bc440735f15b9d1a495acf061dbb5a5a2893a0ed8d7c";

#[derive(Debug, Clone)]
pub struct ResolvedPaths {
    pub tools_dir: PathBuf,
    pub tools_source: ToolsSource,
    pub ltk_host_exe: PathBuf,
    pub ltk_dll_path: PathBuf,
    pub library_dir: PathBuf,
    pub mods_dir: PathBuf,
    pub overlay_dir: PathBuf,
    pub state_dir: PathBuf,
    pub game_dir: PathBuf,

    pub mod_roots: Vec<dekan_core::mods::ModRoot>,

    pub custom_mods_root: PathBuf,
}

impl ResolvedPaths {
    pub fn discover() -> Self {
        let app_state_dir =
            state_dir().unwrap_or_else(|_| std::env::temp_dir().join("Dekan_state"));
        let app_data_dir = data_dir().unwrap_or_else(|_| std::env::temp_dir().join("Dekan"));
        Self::discover_in(app_state_dir, app_data_dir)
    }

    pub fn discover_in(app_state_dir: PathBuf, app_data_dir: PathBuf) -> Self {
        let candidate_tools_dirs = tools_dir_candidates(&app_data_dir);
        let (tools_dir, source) = resolve_tools_dir(&candidate_tools_dirs);

        match source {
            ToolsSource::Own => {
                info!(tools = %tools_dir.display(), "Injection tools found in Dekan's own folder");
            }
            ToolsSource::Missing => {
                error!(
                    expected = %tools_dir.display(),
                    "Injection tools not found. Place ltk_patcher_host.exe and ltk_patcher_dll.dll \
                     in that folder — no skin can be injected until then"
                );
            }
        }

        let ltk_host_exe = tools_dir.join("ltk_patcher_host.exe");
        let ltk_dll_path = tools_dir.join("ltk_patcher_dll.dll");

        let mut candidate_library_dirs =
            vec![app_data_dir.join("library"), app_data_dir.join("skins")];
        if let Some(tool_lib) = tools_dir.parent().map(|p| p.join("library")) {
            candidate_library_dirs.push(tool_lib);
        }

        let library_dir = candidate_library_dirs
            .iter()
            .find(|p| p.is_dir())
            .cloned()
            .unwrap_or_else(|| candidate_library_dirs[0].clone());

        let game_dir = match dekan_platform::paths::discover_game_dir() {
            Some(dir) => dir,
            None => {
                warn!(
                    "League install not found yet (client closed and no install registered by the \
                     Riot Client); it will be looked up again when a skin is prepared"
                );
                PathBuf::new()
            }
        };

        let overlay_dir = app_data_dir.join("overlay");
        let mods_dir = app_data_dir.join("mods");
        let custom_mods_root = dekan_app::mods_store::dekan_mods_root(&app_data_dir);
        let mod_roots = dekan_app::mods_store::mod_roots(&app_data_dir);

        Self {
            tools_dir,
            tools_source: source,
            ltk_host_exe,
            ltk_dll_path,
            library_dir,
            mods_dir,
            overlay_dir,
            state_dir: app_state_dir,
            game_dir,
            mod_roots,
            custom_mods_root,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsSource {
    Own,

    Missing,
}

fn resolve_tools_dir(candidates: &[PathBuf]) -> (PathBuf, ToolsSource) {
    let complete = |dir: &PathBuf| {
        dir.join("ltk_patcher_host.exe").is_file() && dir.join("ltk_patcher_dll.dll").is_file()
    };

    if let Some(dir) = candidates.iter().find(|dir| complete(dir)) {
        info!(tools = %dir.display(), "Injection backend (LTK host + DLL) found");
        return (dir.clone(), ToolsSource::Own);
    }

    (
        candidates.first().cloned().unwrap_or_default(),
        ToolsSource::Missing,
    )
}

pub fn required_tool_files(paths: &ResolvedPaths) -> Vec<PathBuf> {
    vec![paths.ltk_host_exe.clone(), paths.ltk_dll_path.clone()]
}

pub fn tools_ready(paths: &ResolvedPaths) -> bool {
    required_tool_files(paths).iter().all(|file| file.is_file())
}

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

    fn tools_ready(&self) -> bool {
        tools_ready(&self.paths)
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

                        if !self.tools_ready() {
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

                        if armed.is_none() && !self.tools_ready() {
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
                    } else if state.phase.is_champ_select() && !injected_session {
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

                        let divergence = armed.as_ref().and_then(|a| {
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
            ..
        } = key;

        info!(
            champ_id,
            entry_id = ?entry_id,
            mods_fingerprint = mods,
            "Preparing the overlay for the chosen skin and mods, before the game starts"
        );

        let registration = async {
            match entry_id {
                Some(_) if is_classic(champ_id) => None,
                Some(entry_id) => self.register_in_champ_select(champ_id, entry_id).await,
                None => None,
            }
        };
        let (armed, lcu_skin) = tokio::join!(self.build_and_arm(key), registration);
        let (overlay, build) = armed?;

        let game_already_running = matches!(
            ProcessFinder::find_process_by_name(GAME_PROCESS_NAME),
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

        let discovery =
            tokio::task::spawn_blocking(|| dekan_lcu::lockfile::Lockfile::discover(None)).await;

        let lockfile = match discovery {
            Ok(Ok(lockfile)) => lockfile,
            Ok(Err(e)) => {
                warn!(error = %e, "Rule #1 re-read skipped: no lockfile; using the published state");
                return from_state;
            }
            Err(e) => {
                warn!(error = %e, "Rule #1 re-read skipped: lockfile discovery task failed");
                return from_state;
            }
        };

        let client = match dekan_lcu::client::LcuClient::new(
            &lockfile,
            dekan_lcu::client::DEFAULT_LCU_TIMEOUT,
        ) {
            Ok(client) => client,
            Err(e) => {
                warn!(error = %e, "Rule #1 re-read skipped: LCU client unavailable");
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

        let (target, mods, party) = {
            let state = self.state_rx.borrow();
            let (party, _) = party_skins(&state);
            (
                state.overlay_target.clone(),
                state.mods.clone(),
                dekan_core::party::party_fingerprint(&party),
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
                if dekan_core::selection::SelectionMode::is_base_skin(entry_id, champ_id) {
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
            entry_id = armed.key.entry_id,
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
        let mut game_tid = None;
        let discovery_started = tokio::time::Instant::now();
        let max_wait = Duration::from_secs(60);

        while discovery_started.elapsed() < max_wait && !token.is_cancelled() {
            if let Ok(Some(pid)) = ProcessFinder::find_process_by_name(GAME_PROCESS_NAME) {
                if let Ok(Some(tid)) = ProcessFinder::find_first_thread_id(pid) {
                    game_pid = Some(pid);
                    game_tid = Some(tid);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        let (pid, tid) = match (game_pid, game_tid) {
            (Some(p), Some(t)) => (p, t),
            _ => {
                warn!("Timed out waiting for 'League of Legends.exe' game process to spawn");
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
            tid,
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
        let outcome = pipeline.execute(&mods, pid, tid).await;

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

    async fn collect_mods(&self, key: ArmKey) -> Option<Vec<String>> {
        if is_classic(key.champ_id) {
            return self.classic_mods(key).await;
        }

        let mut mods = match key.entry_id {
            Some(entry_id) => self.prepare_mods(key.champ_id, entry_id).await?,
            None => Vec::new(),
        };

        if key.mods != 0 {
            let selection = self.state_rx.borrow().mods.clone();
            if selection.fingerprint(Some(key.champ_id)) != key.mods {
                debug!(
                    champ_id = key.champ_id,
                    "Mod selection changed since the build was scheduled"
                );
            }
            let roots = self.paths.mod_roots.clone();
            let mods_dir = self.paths.mods_dir.clone();
            let game_dir = self.effective_game_dir(None);
            let champion = Some(key.champ_id);
            let staged = tokio::task::spawn_blocking(move || {
                let catalog = dekan_core::mods::scan_catalog(&roots, champion, &|_| true);
                let staged = dekan_app::mods_store::stage_selected(
                    &catalog, &selection, champion, &mods_dir,
                );

                drop_incompatible_mods(staged, &mods_dir, &game_dir)
            })
            .await;
            match staged {
                Ok(staged) => mods.extend(staged),
                Err(e) => {
                    warn!(error = %e, "Custom mod staging task failed; continuing without them")
                }
            }
        }

        if key.party != 0 {
            mods.extend(self.party_mods(key).await);
        }

        if mods.is_empty() {
            warn!(
                champ_id = key.champ_id,
                "Nothing could be prepared for this build; no overlay will be made"
            );
            return None;
        }
        info!(
            champ_id = key.champ_id,
            mods = ?mods,
            "Mods prepared for the overlay build, in merge order"
        );
        Some(mods)
    }

    async fn party_mods(&self, key: ArmKey) -> Vec<String> {
        let (accepted, rejected) = party_skins(&self.state_rx.borrow());
        for (member_id, reason) in &rejected {
            warn!(member_id, reason = ?reason, "Party skin not injected");
        }
        if dekan_core::party::party_fingerprint(&accepted) != key.party {
            debug!("Party skins changed since the build was scheduled; using the current ones");
        }

        let mut staged = Vec::new();
        for (champion_id, entry_id) in accepted {
            if is_classic(champion_id) {
                match self.prepare_classic_party_skin(champion_id, entry_id).await {
                    Some(names) => {
                        info!(champion_id, entry_id, "Classic party skin prepared");
                        staged.extend(names);
                    }
                    None => warn!(
                        champion_id,
                        entry_id, "Could not prepare classic party skin for teammate"
                    ),
                }
            } else {
                match self.prepare_package(champion_id, entry_id, false).await {
                    Some(names) => {
                        info!(champion_id, entry_id, "Party skin prepared");
                        staged.extend(names);
                    }
                    None => warn!(
                        champion_id,
                        entry_id,
                        "A teammate's skin is not in the local library; they will look stock to you"
                    ),
                }
            }
        }
        staged
    }

    async fn prepare_classic_party_skin(
        &self,
        champion_id: u32,
        entry_id: u32,
    ) -> Option<Vec<String>> {
        use dekan_classic::builder::ClassicIdMapper;
        use dekan_classic::generator::{
            ClassicChampion, jade_characters, resolve_alias_with_id, skin_number, slots_for,
        };

        let regular = ClassicIdMapper::normalize_champion_id(champion_id);
        let skin = skin_number(entry_id);
        if skin == 0 {
            return None;
        }
        let slots = slots_for(None);

        let client_alias = match self.lcu_client().await {
            Some(client) => match client.get_champion_assets(regular).await {
                Ok(assets) if !assets.alias.is_empty() => Some(assets.alias),
                Ok(_) => None,
                Err(e) => {
                    debug!(error = %e, regular, "Teammate assets unavailable for Classic alias");
                    None
                }
            },
            None => None,
        };

        let game_dir = self.effective_game_dir(None);
        let library_dir = self.paths.library_dir.join(regular.to_string());
        let hashes = self.hash_table_path();
        let cache = self.paths.state_dir.join("classic_characters.json");
        let cache_dir = self.paths.state_dir.clone();
        let mods_dir = self.paths.mods_dir.clone();
        let classic_alias = self.classic_client_alias(champion_id).await;
        let classic_id = champion_id;
        let built = tokio::task::spawn_blocking(move || {
            let alias = resolve_alias_with_id(
                &game_dir,
                client_alias.as_deref(),
                Some(regular),
                &library_dir,
            )
            .ok_or_else(|| dekan_classic::error::ClassicError::ChampionNotFound {
                alias: format!("no WAD alias found for teammate champion {regular}"),
            })?;
            let classic_alias = classic_alias
                .or_else(|| dekan_classic::client_data::champion_alias(&game_dir, classic_id));
            let champion = ClassicChampion::open(&game_dir, &alias)?
                .with_client_character(classic_alias.as_deref());
            let mut known = jade_characters(&hashes, &cache);
            if known.is_empty() {
                known = champion.jade_names_from_bins_cached(&cache_dir);
            }
            champion.build_mod(skin, &slots, &known, &mods_dir)
        })
        .await;

        match built {
            Ok(Ok(folder)) => {
                info!(champion_id, regular, skin, %folder, "Teammate classic mod ready");
                Some(vec![folder])
            }
            Ok(Err(e)) => {
                warn!(champion_id, regular, skin, error = %e, "Could not build teammate classic mod");
                None
            }
            Err(e) => {
                error!(champion_id, error = %e, "Teammate classic build task failed");
                None
            }
        }
    }

    async fn classic_client_alias(&self, classic_id: u32) -> Option<String> {
        let client = self.lcu_client().await?;
        match client.get_champion_assets(classic_id).await {
            Ok(assets) if assets.alias.to_ascii_lowercase().starts_with("jade_") => {
                Some(assets.alias)
            }
            Ok(_) => None,
            Err(e) => {
                debug!(error = %e, classic_id, "Classic champion assets unavailable from the client");
                None
            }
        }
    }

    async fn classic_mods(&self, key: ArmKey) -> Option<Vec<String>> {
        use dekan_classic::builder::ClassicIdMapper;
        use dekan_classic::generator::{
            ClassicChampion, jade_characters, resolve_alias_with_id, skin_number, slots_for,
        };

        let Some(entry_id) = key.entry_id else {
            debug!(
                champ_id = key.champ_id,
                "Rift Classic takes no custom mod; nothing to build"
            );
            return None;
        };
        let regular = ClassicIdMapper::normalize_champion_id(key.champ_id);
        let skin = skin_number(entry_id);
        let slots = slots_for(key.classic_slot);

        let client_alias = match self.lcu_client().await {
            Some(client) => match client.get_champion_assets(regular).await {
                Ok(assets) if !assets.alias.is_empty() => Some(assets.alias),
                Ok(_) => None,
                Err(e) => {
                    debug!(error = %e, regular, "Champion assets unavailable for the Classic alias");
                    None
                }
            },
            None => None,
        };

        let game_dir = self.effective_game_dir(None);
        let library_dir = self.paths.library_dir.join(regular.to_string());
        let hashes = self.hash_table_path();
        let cache = self.paths.state_dir.join("classic_characters.json");
        let cache_dir = self.paths.state_dir.clone();
        let mods_dir = self.paths.mods_dir.clone();
        let started = std::time::Instant::now();
        let classic_alias = self.classic_client_alias(key.champ_id).await;
        let classic_id = key.champ_id;
        let built = tokio::task::spawn_blocking(move || {
            let alias = resolve_alias_with_id(
                &game_dir,
                client_alias.as_deref(),
                Some(regular),
                &library_dir,
            )
            .ok_or_else(|| dekan_classic::error::ClassicError::ChampionNotFound {
                alias: format!("no WAD alias found for champion {regular}"),
            })?;
            let classic_alias = classic_alias
                .or_else(|| dekan_classic::client_data::champion_alias(&game_dir, classic_id));
            let champion = ClassicChampion::open(&game_dir, &alias)?
                .with_client_character(classic_alias.as_deref());

            let mut known = jade_characters(&hashes, &cache);
            if known.is_empty() {
                known = champion.jade_names_from_bins_cached(&cache_dir);
            }
            champion.build_mod(skin, &slots, &known, &mods_dir)
        })
        .await;

        match built {
            Ok(Ok(folder)) => {
                info!(
                    champ_id = key.champ_id,
                    regular,
                    skin,
                    slots = ?slots_for(key.classic_slot),
                    elapsed_ms = started.elapsed().as_millis(),
                    folder = %folder,
                    "Rift Classic mod ready"
                );
                Some(vec![folder])
            }
            Ok(Err(e)) => {
                warn!(
                    champ_id = key.champ_id,
                    regular,
                    skin,
                    error = %e,
                    "Rift Classic skin cannot be shown; nothing will be injected"
                );
                set_injection_status(
                    &self.state_tx,
                    InjectionStatus::Failed {
                        error: format!("Rift Classic: {e}"),
                    },
                );
                None
            }
            Err(e) => {
                error!(error = %e, "Rift Classic generation task failed");
                None
            }
        }
    }

    fn hash_table_path(&self) -> PathBuf {
        self.paths.tools_dir.join("hashes.game.txt")
    }

    async fn prepare_mods(&self, champ_id: u32, entry_id: u32) -> Option<Vec<String>> {
        self.prepare_package(champ_id, entry_id, true).await
    }

    async fn prepare_package(
        &self,
        champ_id: u32,
        entry_id: u32,
        report_failure: bool,
    ) -> Option<Vec<String>> {
        let library_root = self.paths.library_dir.clone();
        let scan = tokio::task::spawn_blocking(move || {
            dekan_core::library::scan_champion(&library_root, champ_id)
        })
        .await;

        let archive_path = match &scan {
            Ok(library) => library.package_for(entry_id).map(Path::to_path_buf),
            Err(e) => {
                warn!(error = %e, "Library scan task failed; falling back to path probing");
                None
            }
        }
        .or_else(|| {
            find_skin_archive(
                std::slice::from_ref(&self.paths.library_dir),
                champ_id,
                entry_id,
            )
        });

        let Some(archive_path) = archive_path else {
            return self
                .prepare_dynamic_skin(champ_id, entry_id, report_failure)
                .await;
        };

        let mod_name = format!("{champ_id}_{entry_id}");
        let target_dir = self.paths.mods_dir.join(&mod_name);

        info!(
            archive = %archive_path.display(),
            mod_name = %mod_name,
            "Found the mod package for the chosen entry; preparing the mod directory"
        );

        if let Err(e) = prepare_mod_directory(&archive_path, &target_dir) {
            error!(error = %e, mod_name = %mod_name, "Could not prepare the mod directory");
            if report_failure {
                set_injection_status(
                    &self.state_tx,
                    InjectionStatus::Failed {
                        error: format!("mod package could not be extracted: {e}"),
                    },
                );
            }
            return None;
        }

        Some(vec![mod_name])
    }

    async fn prepare_dynamic_skin(
        &self,
        champ_id: u32,
        entry_id: u32,
        report_failure: bool,
    ) -> Option<Vec<String>> {
        use dekan_classic::generator::{StandardChampion, resolve_alias_with_id, skin_number};

        let skin = skin_number(entry_id);
        if skin == 0 {
            debug!(champ_id, entry_id, "Base skin selected; nothing to inject");
            return None;
        }

        let game_dir = self.effective_game_dir(None);
        if !game_dir.is_dir() {
            warn!(
                champ_id,
                entry_id, "Game directory not found; cannot dynamically generate skin"
            );
            if report_failure {
                set_injection_status(
                    &self.state_tx,
                    InjectionStatus::Failed {
                        error: "Game directory not found".into(),
                    },
                );
            }
            return None;
        }

        let assets = match self.lcu_client().await {
            Some(client) => match client.get_champion_assets(champ_id).await {
                Ok(assets) => Some(assets),
                Err(e) => {
                    debug!(
                        error = %e,
                        champ_id,
                        "Champion assets unavailable for dynamic skin generation"
                    );
                    None
                }
            },
            None => None,
        };
        let client_alias = assets
            .as_ref()
            .map(|a| a.alias.clone())
            .filter(|alias| !alias.is_empty());
        let base_skin = assets
            .as_ref()
            .and_then(|a| a.base_skin_of(entry_id))
            .map(skin_number);

        let library_dir = self.paths.library_dir.join(champ_id.to_string());
        let mods_dir = self.paths.mods_dir.clone();
        let cache_dir = self.paths.state_dir.clone();
        let built = tokio::task::spawn_blocking(move || {
            let alias = resolve_alias_with_id(
                &game_dir,
                client_alias.as_deref(),
                Some(champ_id),
                &library_dir,
            )
            .ok_or_else(|| dekan_classic::error::ClassicError::ChampionNotFound {
                alias: format!("no WAD alias found for champion {champ_id}"),
            })?;
            let champion = StandardChampion::open(&game_dir, &alias)?
                .with_cache_dir(&cache_dir)
                .with_options(generation_options());
            champion.build_mod(skin, base_skin, &mods_dir)
        })
        .await;

        match built {
            Ok(Ok(folder)) => {
                info!(
                    champ_id,
                    entry_id,
                    folder = %folder,
                    "Dynamic skin mod generated directly from installed game WAD"
                );
                Some(vec![folder])
            }
            Ok(Err(e)) => {
                warn!(
                    champ_id,
                    entry_id,
                    error = %e,
                    "Could not generate dynamic skin from installed game WAD"
                );
                if report_failure {
                    set_injection_status(
                        &self.state_tx,
                        InjectionStatus::Failed {
                            error: format!("Dynamic skin generation failed: {e}"),
                        },
                    );
                }
                None
            }
            Err(e) => {
                error!(champ_id, error = %e, "Dynamic skin generation task failed");
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

    async fn lcu_client(&self) -> Option<dekan_lcu::client::LcuClient> {
        let discovery =
            tokio::task::spawn_blocking(|| dekan_lcu::lockfile::Lockfile::discover(None)).await;
        let lockfile = match discovery {
            Ok(Ok(lockfile)) => lockfile,
            Ok(Err(e)) => {
                warn!(error = %e, "No LCU lockfile; the skin cannot be registered in champ select");
                return None;
            }
            Err(e) => {
                warn!(error = %e, "Lockfile discovery task failed");
                return None;
            }
        };
        match dekan_lcu::client::LcuClient::new(&lockfile, dekan_lcu::client::DEFAULT_LCU_TIMEOUT)
        {
            Ok(client) => Some(client),
            Err(e) => {
                warn!(error = %e, "Could not build the LCU client");
                None
            }
        }
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
            ltk_host_hash: AUDITED_LTK_HOST_HASH.into(),
            ltk_dll_path: self.paths.ltk_dll_path.clone(),
            ltk_dll_hash: AUDITED_LTK_DLL_HASH.into(),
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

const GAME_PROCESS_NAME: &str = "League of Legends.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArmKey {
    champ_id: u32,

    entry_id: Option<u32>,

    mods: u64,

    classic_slot: Option<u32>,

    party: u64,
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

fn generation_options() -> dekan_classic::generator::GenerationOptions {
    let set_to = |name: &str, value: &str| {
        std::env::var(name).is_ok_and(|v| v.trim().eq_ignore_ascii_case(value))
    };
    let options = dekan_classic::generator::GenerationOptions {
        graph_in_slot0: set_to(dekan_core::env::SKIN_GRAPH, "slot0"),
        chroma_keeps_classification: set_to(dekan_core::env::CHROMA_CLASSIFICATION, "source"),
    };
    if options != dekan_classic::generator::GenerationOptions::default() {
        info!(?options, "Skin generation test variant active");
    }
    options
}

fn is_classic(champ_id: u32) -> bool {
    dekan_classic::builder::ClassicIdMapper::is_classic_champion(champ_id)
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
    })
}

fn drop_incompatible_mods(staged: Vec<String>, mods_dir: &Path, game_dir: &Path) -> Vec<String> {
    let Ok(game) = dekan_inject::overlay_builder::get_or_index_game(game_dir) else {
        return staged;
    };
    let game_hashes = dekan_inject::mod_compat::game_hash_set(&game);
    staged
        .into_iter()
        .filter(|name| {
            let wad_dir = mods_dir.join(name).join("WAD");
            let Ok(entries) = std::fs::read_dir(&wad_dir) else {
                return true;
            };
            let mut dangling = Vec::new();
            for wad in entries.flatten().map(|e| e.path()) {
                if wad.extension().is_none_or(|ext| ext != "client") || !wad.is_file() {
                    continue;
                }
                if let Ok(compat) = dekan_inject::mod_compat::check_wad(&wad, &game_hashes) {
                    dangling.extend(compat.dangling);
                }
            }
            if dangling.is_empty() {
                true
            } else {
                warn!(
                    mod_name = %name,
                    dangling = ?dangling,
                    "Custom mod is incompatible with the installed patch (its PROP links a .bin the \
                     game no longer has); dropped so it does not crash the game on the loading screen"
                );
                false
            }
        })
        .collect()
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
        .filter(|entry_id| {
            !dekan_core::selection::SelectionMode::is_base_skin(*entry_id, champ_id)
        });

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

fn find_skin_archive(candidate_roots: &[PathBuf], champ_id: u32, skin_id: u32) -> Option<PathBuf> {
    for root in candidate_roots {
        if !root.is_dir() {
            continue;
        }

        let p1 = root
            .join(champ_id.to_string())
            .join(skin_id.to_string())
            .join(format!("{skin_id}.fantome"));
        if p1.is_file() {
            return Some(p1);
        }

        let p2 = root
            .join(champ_id.to_string())
            .join(format!("{skin_id}.fantome"));
        if p2.is_file() {
            return Some(p2);
        }

        let p3 = root.join(format!("{champ_id}_{skin_id}.fantome"));
        if p3.is_file() {
            return Some(p3);
        }

        let p4 = root.join(format!("{skin_id}.fantome"));
        if p4.is_file() {
            return Some(p4);
        }

        let p5 = root.join(champ_id.to_string()).join(skin_id.to_string());
        if p5.is_dir() {
            return Some(p5);
        }
    }
    None
}

fn extracted_mod_is_complete(mod_dir: &Path) -> bool {
    let wad_dir = ["WAD", "wad"]
        .iter()
        .map(|name| mod_dir.join(name))
        .find(|path| path.is_dir());

    let Some(wad_dir) = wad_dir else {
        return false;
    };

    std::fs::read_dir(wad_dir)
        .map(|entries| entries.flatten().any(|entry| entry.path().is_file()))
        .unwrap_or(false)
}

fn prepare_mod_directory(archive_path: &Path, target_dir: &Path) -> std::io::Result<()> {
    if target_dir.exists() {
        if extracted_mod_is_complete(target_dir) {
            return Ok(());
        }

        warn!(
            mod_dir = %target_dir.display(),
            "Extracted mod directory has no WAD; discarding it and extracting again"
        );
        std::fs::remove_dir_all(target_dir)?;
    }

    if archive_path.is_dir() {
        copy_dir_all(archive_path, target_dir)?;
        return Ok(());
    }

    std::fs::create_dir_all(target_dir)?;
    let file = std::fs::File::open(archive_path)?;
    let mut zip_archive =
        zip::ZipArchive::new(file).map_err(|e| std::io::Error::other(format!("zip error: {e}")))?;

    for i in 0..zip_archive.len() {
        let mut zip_file = zip_archive
            .by_index(i)
            .map_err(|e| std::io::Error::other(format!("zip error: {e}")))?;
        let outpath = match zip_file.enclosed_name() {
            Some(path) => target_dir.join(path),
            None => continue,
        };

        if zip_file.is_dir() {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            let mut out = std::fs::File::create(&outpath)?;
            std::io::copy(&mut zip_file, &mut out)?;
        }
    }

    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "trigger_tests.rs"]
mod tests;
