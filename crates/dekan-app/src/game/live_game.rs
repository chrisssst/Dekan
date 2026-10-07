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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Player {
    pub team: String,
    pub champion: String,
    pub skin_id: i64,
    pub skin_name: String,
    pub local: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GameEvent {
    pub id: i64,
    pub name: String,
    pub time: f64,
    pub killer: Option<String>,
    pub victim: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AllGameData {
    pub game_time: f64,
    pub mode: String,
    pub map: i64,
    pub players: Vec<Player>,
    pub events: Vec<GameEvent>,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn player_key(v: &Value) -> String {
    let riot = text(v, "riotId");
    if riot.is_empty() {
        text(v, "summonerName")
    } else {
        riot
    }
}

#[must_use]
pub fn parse_all_game_data(json: &Value) -> Option<AllGameData> {
    let local = json.get("activePlayer").map(player_key).unwrap_or_default();
    let raw_players = json.get("allPlayers")?.as_array()?;
    let mut champion_of = BTreeMap::new();
    let mut players = Vec::with_capacity(raw_players.len());
    for p in raw_players {
        let key = player_key(p);
        let champion = text(p, "championName");
        champion_of.insert(key.clone(), champion.clone());
        champion_of.insert(text(p, "summonerName"), champion.clone());
        players.push(Player {
            team: text(p, "team"),
            champion,
            skin_id: p.get("skinID").and_then(Value::as_i64).unwrap_or(-1),
            skin_name: text(p, "skinName"),
            local: !local.is_empty() && key == local,
        });
    }
    let as_champion = |name: Option<&str>| {
        name.filter(|n| !n.is_empty()).map(|n| {
            champion_of
                .get(n)
                .cloned()
                .unwrap_or_else(|| "non-champion".to_owned())
        })
    };
    let events = json
        .pointer("/events/Events")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|e| GameEvent {
                    id: e.get("EventID").and_then(Value::as_i64).unwrap_or(-1),
                    name: text(e, "EventName"),
                    time: e.get("EventTime").and_then(Value::as_f64).unwrap_or(0.0),
                    killer: as_champion(e.get("KillerName").and_then(Value::as_str)),
                    victim: as_champion(e.get("VictimName").and_then(Value::as_str)),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(AllGameData {
        game_time: json
            .pointer("/gameData/gameTime")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        mode: json
            .pointer("/gameData/gameMode")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        map: json
            .pointer("/gameData/mapNumber")
            .and_then(Value::as_i64)
            .unwrap_or(-1),
        players,
        events,
    })
}

fn describe(p: &Player) -> String {
    format!("{} {}:{} ({})", p.team, p.champion, p.skin_id, p.skin_name)
}

#[must_use]
pub fn game_time_for(timeline: &[(SystemTime, f64)], when: SystemTime) -> Option<f64> {
    let (at, game_time) = timeline.iter().rev().find(|(at, _)| *at <= when)?;
    let since = when.duration_since(*at).ok()?.as_secs_f64();
    Some(game_time + since)
}

#[must_use]
pub fn format_game_time(seconds: f64) -> String {
    let whole = seconds.max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GameLogFacts {
    pub roster: Vec<(String, u32, bool)>,
    pub errors: BTreeMap<String, usize>,
}

#[must_use]
pub fn parse_game_log(content: &str) -> GameLogFacts {
    let mut facts = GameLogFacts::default();
    let mut seen = BTreeSet::new();
    for line in content.lines() {
        let body = line.split_once('|').map_or(line, |(_, rest)| rest).trim();
        if body.contains("CONNECTION READY") {
            let champion = body
                .split_once("Champion(")
                .and_then(|(_, r)| r.split_once(')'))
                .map(|(c, _)| c.to_owned());
            let skin = body
                .split_once("SkinID(")
                .and_then(|(_, r)| r.split_once(')'))
                .and_then(|(s, _)| s.parse::<u32>().ok());
            if let (Some(champion), Some(skin)) = (champion, skin) {
                let local = body.contains("**LOCAL**");
                if seen.insert((champion.clone(), skin, local)) {
                    facts.roster.push((champion, skin, local));
                }
            }
        } else if let Some(message) = body
            .strip_prefix("ERROR|")
            .or_else(|| body.strip_prefix("FATAL|"))
        {
            *facts.errors.entry(message.trim().to_owned()).or_default() += 1;
        }
    }
    facts
}

fn newest_game_log(logs_dir: &Path, since: SystemTime) -> Option<PathBuf> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for dir in std::fs::read_dir(logs_dir).ok()?.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if !path.to_string_lossy().ends_with("_r3dlog.txt") {
                continue;
            }
            let Ok(modified) = file.metadata().and_then(|m| m.modified()) else {
                continue;
            };
            if modified >= since && newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                newest = Some((modified, path));
            }
        }
    }
    newest.map(|(_, path)| path)
}

#[must_use]
pub fn game_logs_dir(game_dir: &Path) -> Option<PathBuf> {
    game_dir
        .parent()
        .map(|install| install.join("Logs").join("GameLogs"))
}

fn report_game_log(game_dir: &Path, since: SystemTime) {
    let Some(dir) = game_logs_dir(game_dir) else {
        return;
    };
    let Some(path) = newest_game_log(&dir, since) else {
        info!(dir = %dir.display(), "No game log was written for this match");
        return;
    };
    let content = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() <= MAX_GAME_LOG_BYTES => std::fs::read(&path),
        Ok(meta) => {
            warn!(file = %path.display(), bytes = meta.len(), "Game log too large to read; skipped");
            return;
        }
        Err(e) => Err(e),
    };
    let content = match content {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => {
            warn!(file = %path.display(), error = %e, "Game log could not be read");
            return;
        }
    };
    let facts = parse_game_log(&content);
    info!(
        file = %path.display(),
        roster = ?facts.roster.iter().map(|(c, s, l)| format!("{c}:{s}{}", if *l { " (you)" } else { "" })).collect::<Vec<_>>(),
        distinct_errors = facts.errors.len(),
        "Game log: skins the game loaded for this match"
    );
    for (message, count) in &facts.errors {
        if message.contains("FATAL")
            || message.contains("mount failed")
            || message.contains("corrupt")
        {
            warn!(count, message = %message, "Game log error");
        } else {
            info!(count, message = %message, "Game log error");
        }
    }
}

#[must_use]
pub fn match_screenshots(
    install_dir: &Path,
    since: SystemTime,
    until: SystemTime,
) -> Vec<(SystemTime, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(install_dir.join("Screenshots")) else {
        return Vec::new();
    };
    let mut shots: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let image = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("png") || x.eq_ignore_ascii_case("jpg"));
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            (image && modified >= since && modified <= until).then_some((modified, path))
        })
        .collect();
    shots.sort();
    if shots.len() > MAX_MATCH_SCREENSHOTS {
        shots.drain(..shots.len() - MAX_MATCH_SCREENSHOTS);
    }
    shots
}

fn prune_exports(logs_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(logs_dir) else {
        return;
    };
    let mut exports: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("dekan-diagnostics-") && n.ends_with(".zip"))
        })
        .collect();
    exports.sort();
    if exports.len() <= KEPT_AUTOMATIC_EXPORTS {
        return;
    }
    for old in &exports[..exports.len() - KEPT_AUTOMATIC_EXPORTS] {
        if let Err(e) = std::fs::remove_file(old) {
            debug!(file = %old.display(), error = %e, "Old diagnostics export not removed");
        }
    }
}

fn close_match(
    game_dir: &Path,
    logs_dir: Option<&Path>,
    since: SystemTime,
    timeline: &[(SystemTime, f64)],
) {
    report_game_log(game_dir, since);
    let until = SystemTime::now();
    let shots = game_dir
        .parent()
        .map(|install| match_screenshots(install, since, until))
        .unwrap_or_default();
    for (taken, path) in &shots {
        info!(
            file = %path.display(),
            game_time = ?game_time_for(timeline, *taken).map(format_game_time),
            "Screenshot taken during the match"
        );
    }
    let Some(logs_dir) = logs_dir else {
        return;
    };
    let images: Vec<PathBuf> = shots.into_iter().map(|(_, p)| p).collect();
    match crate::control_panel::export_diagnostics(logs_dir, until, &images) {
        Ok(zip) => info!(
            file = %zip.display(),
            screenshots = images.len(),
            "Match diagnostics exported automatically"
        ),
        Err(e) => warn!(error = %e, "Match diagnostics could not be exported"),
    }
    prune_exports(logs_dir);
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

#[cfg(test)]
#[path = "live_game_tests.rs"]
mod tests;
