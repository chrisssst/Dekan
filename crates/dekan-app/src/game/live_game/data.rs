use super::*;

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

pub(super) fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn player_key(v: &Value) -> String {
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

pub(super) fn describe(p: &Player) -> String {
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
