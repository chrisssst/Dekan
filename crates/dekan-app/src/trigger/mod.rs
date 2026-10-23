use std::path::{Path, PathBuf};
use std::time::Duration;

use dekan_core::phase::GamePhase;
use dekan_core::state::{InjectionStatus, StateReceiver, StateSender, set_injection_status};
use dekan_inject::overlay::OverlayConfig;
use dekan_inject::pipeline::{InjectionPipeline, PipelineConfig};
use dekan_platform::paths::{data_dir, state_dir, tools_dir_candidates};
use dekan_platform::process::ProcessFinder;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

mod arm;
mod prepare;
mod support;

use arm::{arm_key, arming};
use arm_key::{
    ArmDecision, ArmKey, ArmRequest, WantedSkin, arm_decision, arms_in_lobby, build_key,
    is_classic, lobby_mods_fingerprint, lobby_overlay_serves, party_skins, should_disarm,
    wanted_skin_when,
};
use arming::{ArmedPatcher, Arming, next_armed, resume_for_reconnect};
pub use paths::{ResolvedPaths, ToolsSource, required_tool_files, tools_ready};
use prepare::paths;
use support::{match_ended, note_wanted, prefer_lazy_wad_checks};

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
        let mut failed_arm: Option<ArmKey> = None;
        let mut last_wanted: Option<WantedSkin> = None;
        let mut tools_missing_reported = false;
        let mut injection_off_reported = false;

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

                        if !dekan_platform::preferences::INJECTION.is_enabled() {
                            debug!(
                                entry_id = ?request.key.entry_id,
                                "Skin injection is turned off; the pending patcher build is dropped"
                            );
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
                        if let Some(running) = active_overlay.take() {
                            info!(
                                entry_id = ?request.key.entry_id,
                                "A new pick arrived; stopping the patcher that served the previous one"
                            );
                            running.shutdown().await;
                        }
                        arming = Some(Arming {
                            key: request.key(),
                            task: Box::pin(self.arm_patcher(request)),
                        });
                    }
                }

                result = next_armed(&mut arming) => {
                    if let (None, Some(finished)) = (&result, arming.take()) {
                        warn!(
                            champ_id = finished.key.champ_id,
                            entry_id = ?finished.key.entry_id,
                            "The patcher could not be armed; this pick is not retried until the selection changes, and the game-start path will build against the running game"
                        );
                        failed_arm = Some(finished.key);
                    }
                    arming = None;
                    armed = result;
                    lcu_divergence_seen = None;
                }

                res = state_rx.changed() => {
                    if res.is_err() {
                        break;
                    }

                    let state = state_rx.borrow().clone();

                    if !dekan_platform::preferences::INJECTION.is_enabled() {
                        if let Some(overlay) = active_overlay.take() {
                            info!(phase = ?state.phase, "Skin injection turned off; stopping the running patcher so a reconnect loads the game's own files");
                            overlay.shutdown().await;
                        }
                    }

                    if state.phase == GamePhase::Reconnect && injected_session {
                        injected_session = false;
                        let wanted = wanted_skin_when(&state, dekan_platform::preferences::INJECTION.is_enabled());
                        (armed, active_overlay) = resume_for_reconnect(active_overlay.take(), wanted);
                        info!(kept = ?armed.as_ref().map(|a| a.key.entry_id), "Game left for the reconnect screen; the skin can be changed before reconnecting");
                    }

                    let reconnecting = state.phase == GamePhase::Reconnect;
                    if state.phase.is_in_game() && !reconnecting && !injected_session {
                        injected_session = true;
                        let last_second = pending_arm.take();

                        if !dekan_platform::preferences::INJECTION.is_enabled() {
                            if let Some(abandoned) = arming.take() {
                                info!(entry_id = ?abandoned.key.entry_id, "Skin injection turned off; abandoning the patcher build");
                            }
                            if let Some(stale) = armed.take() {
                                stale.overlay.shutdown().await;
                            }
                            info!(phase = ?state.phase, "Game started with skin injection turned off; nothing is injected this match");
                            continue;
                        }

                        if arming.is_none() {
                            arming = self.arm_last_second_pick(last_second, &mut armed).await;
                        }
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
                    } else if (state.phase.is_champ_select() || arms_in_lobby(&state) || reconnecting) && !injected_session {
                        let injection_enabled = dekan_platform::preferences::INJECTION.is_enabled();
                        if !injection_enabled && !injection_off_reported {
                            injection_off_reported = true;
                            info!("Skin injection is turned off; this pick will not be injected");
                        }
                        let wanted = wanted_skin_when(&state, injection_enabled);
                        note_wanted(&mut last_wanted, wanted, state.phase);
                        if failed_arm.is_some_and(|failed| match wanted {
                            WantedSkin::Nothing => true,
                            WantedSkin::Skin(key) => key != failed,
                            WantedSkin::Unknown => false,
                        }) {
                            failed_arm = None;
                        }

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

                        let current = armed.as_ref().map(|a| a.key).or_else(|| arming.as_ref().map(|a| a.key));
                        match arm_decision(wanted, current) {
                            ArmDecision::Settled => pending_arm = None,
                            ArmDecision::Schedule(request) if failed_arm == Some(request.key) => {
                                pending_arm = None;
                            }
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
                            || injection_off_reported
                            || armed.is_some()
                            || arming.is_some()
                            || active_overlay.is_some()
                            || pending_arm.is_some())
                    {
                        debug!("Resetting injection session state for next match");
                        injected_session = false;
                        tools_missing_reported = false;
                        injection_off_reported = false;
                        pending_arm = None;
                        lcu_divergence_seen = None;
                        failed_arm = None;
                        last_wanted = None;
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

    fn effective_game_dir(&self, pid: Option<u32>) -> PathBuf {
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
}

const ARM_DEBOUNCE: Duration = Duration::from_millis(900);

const INITIAL_ARM_DEBOUNCE: Duration = Duration::from_millis(100);

const GAME_PROCESS_POLL: Duration = Duration::from_millis(100);

#[cfg(test)]
mod tests;
