use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use dekan_core::phase::GamePhase;
use dekan_core::state::StateReceiver;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

const LIVE_URL: &str = "https://127.0.0.1:2999/liveclientdata/allgamedata";
const POLL_EVERY: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const GAME_LOG_SETTLE: Duration = Duration::from_secs(8);
const MAX_GAME_LOG_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MATCH_SCREENSHOTS: usize = 12;
const KEPT_AUTOMATIC_EXPORTS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct LiveSnapshot {
    pub observed_at: SystemTime,
    pub game_time: f64,
    pub champion: String,
    pub skin_id: i64,
    pub skin_name: String,
}

type MatchHook = Box<dyn Fn(bool) + Send + Sync>;

#[derive(Clone, Default)]
pub struct LiveGame {
    snapshot: Arc<Mutex<Option<LiveSnapshot>>>,
    on_match: Arc<Mutex<Option<MatchHook>>>,
}

impl std::fmt::Debug for LiveGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveGame")
            .field("snapshot", &self.latest())
            .finish_non_exhaustive()
    }
}

impl LiveGame {
    #[must_use]
    pub fn latest(&self) -> Option<LiveSnapshot> {
        self.snapshot.lock().ok().and_then(|s| s.clone())
    }

    pub fn on_match(&self, hook: MatchHook) {
        if let Ok(mut slot) = self.on_match.lock() {
            *slot = Some(hook);
        }
    }

    fn match_changed(&self, playing: bool) {
        if let Ok(slot) = self.on_match.lock() {
            if let Some(hook) = slot.as_ref() {
                hook(playing);
            }
        }
    }

    #[must_use]
    pub fn game_time_at(&self, when: SystemTime) -> Option<f64> {
        let snapshot = self.latest()?;
        game_time_for(&[(snapshot.observed_at, snapshot.game_time)], when)
    }

    fn set(&self, snapshot: Option<LiveSnapshot>) {
        if let Ok(mut slot) = self.snapshot.lock() {
            *slot = snapshot;
        }
    }
}

#[derive(Default)]
struct MatchWatch {
    started: Option<SystemTime>,
    timeline: Vec<(SystemTime, f64)>,
    last_players: Vec<Player>,
    last_event: i64,
    polls: u32,
    answered: u32,
}

impl MatchWatch {
    fn observe(&mut self, data: &AllGameData, live: &LiveGame, now: SystemTime) {
        self.answered += 1;
        self.timeline.push((now, data.game_time));
        if self.last_players.is_empty() {
            info!(
                mode = %data.mode,
                map = data.map,
                game_time = data.game_time,
                players = ?data.players.iter().map(describe).collect::<Vec<_>>(),
                "Live game data: roster and skins as the game reports them"
            );
        } else {
            for now in &data.players {
                let before = self
                    .last_players
                    .iter()
                    .find(|p| p.team == now.team && p.champion == now.champion);
                if let Some(before) =
                    before.filter(|b| b.skin_id != now.skin_id || b.skin_name != now.skin_name)
                {
                    warn!(
                        game_time = data.game_time,
                        local = now.local,
                        from = %describe(before),
                        to = %describe(now),
                        "Live game data: a skin changed during the match"
                    );
                }
            }
        }
        let seen = self.last_event;
        for event in data.events.iter().filter(|e| e.id > seen) {
            info!(
                event = %event.name,
                event_time = event.time,
                killer = ?event.killer,
                victim = ?event.victim,
                "Live game event"
            );
        }
        self.last_event = data.events.iter().map(|e| e.id).fold(seen, i64::max);
        self.last_players = data.players.clone();
        live.set(data.players.iter().find(|p| p.local).map(|p| LiveSnapshot {
            observed_at: now,
            game_time: data.game_time,
            champion: p.champion.clone(),
            skin_id: p.skin_id,
            skin_name: p.skin_name.clone(),
        }));
    }
}

async fn poll(client: &reqwest::Client) -> Result<AllGameData, String> {
    let response = client
        .get(LIVE_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let json: Value = response.json().await.map_err(|e| e.to_string())?;
    parse_all_game_data(&json).ok_or_else(|| "unexpected shape".to_owned())
}

pub async fn run(
    mut state_rx: StateReceiver,
    game_dir: PathBuf,
    logs_dir: Option<PathBuf>,
    live: LiveGame,
    token: CancellationToken,
) {
    let client = match reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .danger_accept_invalid_certs(true)
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            warn!(error = %e, "Live game data off: the local HTTP client could not be created");
            return;
        }
    };
    let mut watch: Option<MatchWatch> = None;
    loop {
        let phase = state_rx.borrow_and_update().phase;
        let in_game = match_running(phase, game_alive);
        match (&mut watch, in_game) {
            (None, true) => {
                watch = Some(MatchWatch {
                    started: Some(SystemTime::now()),
                    last_event: -1,
                    ..MatchWatch::default()
                });
                info!("Match started; reading the game's local live data every 5 s");
                live.match_changed(true);
            }
            (Some(current), false) => {
                info!(
                    polls = current.polls,
                    answered = current.answered,
                    "Match ended; live game data stopped"
                );
                live.match_changed(false);
                live.set(None);
                let since = current.started.unwrap_or(SystemTime::UNIX_EPOCH);
                let timeline = std::mem::take(&mut current.timeline);
                watch = None;
                tokio::select! {
                    _ = token.cancelled() => break,
                    () = tokio::time::sleep(GAME_LOG_SETTLE) => {}
                }
                let dir = game_dir.clone();
                let logs = logs_dir.clone();
                if let Err(e) = tokio::task::spawn_blocking(move || {
                    close_match(&dir, logs.as_deref(), since, &timeline)
                })
                .await
                {
                    warn!(error = %e, "Game log reading task failed");
                }
            }
            _ => {}
        }
        if let Some(current) = &mut watch {
            current.polls += 1;
            match poll(&client).await {
                Ok(data) => current.observe(&data, &live, SystemTime::now()),
                Err(e) => debug!(error = %e, "Live game data not available yet"),
            }
        }
        tokio::select! {
            _ = token.cancelled() => break,
            changed = state_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            () = tokio::time::sleep(POLL_EVERY), if watch.is_some() || phase == GamePhase::Reconnect => {}
        }
    }
}

#[must_use]
pub fn match_running(phase: GamePhase, game_alive: impl FnOnce() -> bool) -> bool {
    match phase {
        GamePhase::Reconnect => game_alive(),
        other => other.is_in_game(),
    }
}

fn game_alive() -> bool {
    match dekan_platform::process::ProcessFinder::find_any_process(
        &dekan_platform::game_version::GAME_EXES,
    ) {
        Ok(found) => found.is_some(),
        Err(e) => {
            debug!(error = %e, "Game process lookup failed; the match is treated as still running");
            true
        }
    }
}

mod data;
mod logs;

use data::describe;
pub use data::{
    AllGameData, GameEvent, Player, format_game_time, game_time_for, parse_all_game_data,
};
use logs::close_match;
pub use logs::{GameLogFacts, game_logs_dir, match_screenshots, parse_game_log};

#[cfg(test)]
mod tests;
