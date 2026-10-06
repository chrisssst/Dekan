use super::*;

fn sample() -> Value {
    serde_json::json!({
        "activePlayer": { "riotId": "Me#BR1", "summonerName": "Me" },
        "allPlayers": [
            { "riotId": "Me#BR1", "summonerName": "Me", "championName": "Garen", "skinID": 0, "skinName": "default", "team": "ORDER" },
            { "riotId": "Foe#NA1", "summonerName": "Foe", "championName": "Viego", "skinID": 43, "skinName": "Revenant Reign Viego", "team": "CHAOS" }
        ],
        "events": { "Events": [
            { "EventID": 0, "EventName": "GameStart", "EventTime": 0.03 },
            { "EventID": 1, "EventName": "ChampionKill", "EventTime": 312.5, "KillerName": "Foe#NA1", "VictimName": "Me#BR1" },
            { "EventID": 2, "EventName": "TurretKilled", "EventTime": 600.0, "KillerName": "Turret_T1_C_05_A", "VictimName": "" }
        ]},
        "gameData": { "gameTime": 640.2, "gameMode": "CLASSIC", "mapNumber": 11 }
    })
}

#[test]
fn the_local_player_and_every_skin_are_read_without_names() {
    let data = parse_all_game_data(&sample()).expect("shape");
    assert_eq!((data.mode.as_str(), data.map), ("CLASSIC", 11));
    assert_eq!(data.game_time, 640.2);
    let me = data.players.iter().find(|p| p.local).expect("local player");
    assert_eq!((me.champion.as_str(), me.skin_id), ("Garen", 0));
    assert_eq!(data.players.iter().filter(|p| p.local).count(), 1);
    let described: Vec<String> = data.players.iter().map(describe).collect();
    assert!(
        described
            .iter()
            .all(|d| !d.contains("Foe") && !d.contains("Me#")),
        "{described:?}"
    );
}

#[test]
fn events_name_champions_instead_of_players() {
    let data = parse_all_game_data(&sample()).expect("shape");
    let kill = &data.events[1];
    assert_eq!((kill.name.as_str(), kill.time), ("ChampionKill", 312.5));
    assert_eq!(kill.killer.as_deref(), Some("Viego"));
    assert_eq!(kill.victim.as_deref(), Some("Garen"));
    assert_eq!(data.events[2].killer.as_deref(), Some("non-champion"));
    assert_eq!(data.events[2].victim, None);
}

#[test]
fn a_skin_change_during_the_match_updates_what_is_marked() {
    let live = LiveGame::default();
    let mut watch = MatchWatch {
        last_event: -1,
        ..MatchWatch::default()
    };
    let mut data = parse_all_game_data(&sample()).expect("shape");
    watch.observe(
        &data,
        &live,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000),
    );
    assert_eq!(watch.last_event, 2, "every event is seen once");
    assert_eq!(live.latest().map(|s| s.skin_id), Some(0));
    data.players[0].skin_id = 44;
    data.game_time = 700.0;
    watch.observe(
        &data,
        &live,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_060),
    );
    let latest = live.latest().expect("snapshot");
    assert_eq!((latest.skin_id, latest.game_time), (44, 700.0));
    assert_eq!(watch.answered, 2);
}

#[test]
fn unexpected_answers_are_refused() {
    assert_eq!(parse_all_game_data(&serde_json::json!({})), None);
    assert_eq!(
        parse_all_game_data(&serde_json::json!({"allPlayers": 3})),
        None
    );
}

#[test]
fn the_game_log_gives_the_roster_skins_and_counted_errors_only() {
    let log = "\
000002.149| ALWAYS|  ROST| CONNECTION READY | TeamOrder 0) 'Orianna' - Champion(Orianna) SkinID(20) TeamBuilderRole(NONE) PUUID(aaaa) ConnectionState(Connected)\n\
000002.149| ALWAYS|  ROST| CONNECTION READY | TeamOrder 1) 'Katarina' **LOCAL** - Champion(Katarina) SkinID(0) TeamBuilderRole(NONE) PUUID(bbbb) ConnectionState(Connected)\n\
000010.486|  ERROR| LCUVoiceChatClient: Failed initial voice-fonts GET\n\
000011.000|  ERROR| LCUVoiceChatClient: Failed initial voice-fonts GET\n\
000012.000|  ERROR| ALE-N FATAL ERROR - Installation is corrupt. WadFile mount failed.\n";
    let facts = parse_game_log(log);
    assert_eq!(
        facts.roster,
        vec![
            ("Orianna".to_owned(), 20, false),
            ("Katarina".to_owned(), 0, true)
        ]
    );
    assert_eq!(
        facts
            .errors
            .get("LCUVoiceChatClient: Failed initial voice-fonts GET"),
        Some(&2)
    );
    assert_eq!(facts.errors.len(), 2);
    let flat = format!("{facts:?}");
    assert!(!flat.contains("PUUID") && !flat.contains("aaaa"), "{flat}");
}

#[test]
fn the_game_logs_folder_sits_next_to_the_game_folder() {
    let game = Path::new("D:/Riot Games/League of Legends/Game");
    assert_eq!(
        game_logs_dir(game),
        Some(PathBuf::from(
            "D:/Riot Games/League of Legends/Logs/GameLogs"
        ))
    );
}

#[test]
fn only_logs_written_since_the_match_started_are_read() {
    let root = std::env::temp_dir().join(format!("dekan_game_logs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet
    let old = root.join("2026-09-29T10-00-00");
    std::fs::create_dir_all(&old).expect("dir");
    std::fs::write(old.join("2026-09-29T10-00-00_r3dlog.txt"), "old").expect("old");
    let since = SystemTime::now();
    std::thread::sleep(Duration::from_millis(30));
    let new = root.join("2026-09-30T10-00-00");
    std::fs::create_dir_all(&new).expect("dir");
    std::fs::write(new.join("2026-09-30T10-00-00_r3dlog.txt"), "new").expect("new");
    std::fs::write(new.join("2026-09-30T10-00-00_netlog.txt"), "net").expect("net");
    assert_eq!(
        newest_game_log(&root, since),
        Some(new.join("2026-09-30T10-00-00_r3dlog.txt"))
    );
    assert_eq!(
        newest_game_log(&root, SystemTime::now() + Duration::from_secs(60)),
        None
    );
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

#[test]
fn a_moment_is_turned_into_game_time_from_the_timeline() {
    let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let timeline = vec![(t0, 100.0), (t0 + Duration::from_secs(5), 105.0)];
    assert_eq!(
        game_time_for(&timeline, t0 + Duration::from_secs(7)),
        Some(107.0)
    );
    assert_eq!(
        game_time_for(&timeline, t0 + Duration::from_secs(2)),
        Some(102.0)
    );
    assert_eq!(
        game_time_for(&timeline, t0 - Duration::from_secs(1)),
        None,
        "before the first sample"
    );
    assert_eq!(format_game_time(751.9), "12:31");
    assert_eq!(format_game_time(-3.0), "0:00");
    let live = LiveGame::default();
    live.set(Some(LiveSnapshot {
        observed_at: t0,
        game_time: 60.0,
        champion: "Garen".into(),
        skin_id: 0,
        skin_name: "default".into(),
    }));
    assert_eq!(live.game_time_at(t0 + Duration::from_secs(3)), Some(63.0));
}

#[test]
fn only_screenshots_from_the_match_are_collected() {
    let install = std::env::temp_dir().join(format!("dekan_shots_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&install); // ignore-ok: fixture may not exist yet
    let dir = install.join("Screenshots");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("Screen01.png"), b"old").expect("old");
    std::thread::sleep(Duration::from_millis(30));
    let since = SystemTime::now();
    std::thread::sleep(Duration::from_millis(30));
    std::fs::write(dir.join("Screen02.png"), b"match").expect("match");
    std::fs::write(dir.join("notes.txt"), b"not an image").expect("text");
    let shots = match_screenshots(&install, since, SystemTime::now() + Duration::from_secs(1));
    let names: Vec<_> = shots
        .iter()
        .map(|(_, p)| p.file_name().expect("name").to_owned())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("Screen02.png")]);
    let _ = std::fs::remove_dir_all(&install); // ignore-ok: fixture cleanup
}

#[test]
fn the_match_hook_hears_when_a_match_starts_and_ends() {
    let live = LiveGame::default();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    live.on_match(Box::new(move |playing| {
        if let Ok(mut list) = sink.lock() {
            list.push(playing);
        }
    }));
    live.clone().match_changed(true);
    live.match_changed(false);
    assert_eq!(*seen.lock().expect("lock"), vec![true, false]);
}
