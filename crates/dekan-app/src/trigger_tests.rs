use super::mods::*;
use super::paths::*;
use super::*;

fn champ_select_state(
    champion_id: Option<u32>,
    target: Option<dekan_core::overlay::OverlayTarget>,
) -> dekan_core::state::AppState {
    dekan_core::state::AppState {
        phase: dekan_core::phase::GamePhase::ChampSelect,
        champion_id,
        overlay_target: target,
        ..Default::default()
    }
}

fn target(champion_id: u32, skin_id: u32) -> dekan_core::overlay::OverlayTarget {
    dekan_core::overlay::OverlayTarget {
        champion_id,
        skin_id,
        chroma_id: None,
    }
}

#[tokio::test]
async fn test_match_ended_waits_for_the_match_to_end() {
    use dekan_core::phase::GamePhase;
    use dekan_core::state::{new_state_channel, set_phase};

    let (tx, rx) = new_state_channel();
    set_phase(&tx, GamePhase::InProgress);
    let mut phase_rx = rx.clone();
    let short = Duration::from_millis(50);

    let pending = tokio::time::timeout(short, match_ended(&mut phase_rx)).await;
    assert!(pending.is_err(), "resolved while the game was in progress");

    set_phase(&tx, GamePhase::Reconnect);
    let pending = tokio::time::timeout(short, match_ended(&mut phase_rx)).await;
    assert!(pending.is_err(), "resolved on a reconnect");

    set_phase(&tx, GamePhase::EndOfGame);
    let ended = tokio::time::timeout(short, match_ended(&mut phase_rx)).await;
    assert!(ended.is_ok(), "did not resolve once the match ended");

    assert!(rx.has_changed().unwrap_or(false));

    let (tx, mut closed_rx) = new_state_channel();
    set_phase(&tx, GamePhase::InProgress);
    drop(tx);
    let pending = tokio::time::timeout(short, match_ended(&mut closed_rx)).await;
    assert!(pending.is_err(), "resolved on a closed channel");
}

#[test]
fn test_a_half_extracted_mod_directory_is_rebuilt_not_trusted() {
    let root = std::env::temp_dir().join(format!("dekan_mod_reextract_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture may not exist yet

    std::fs::create_dir_all(&root).expect("fixture root");
    let archive_path = root.join("81069.fantome");
    {
        let file = std::fs::File::create(&archive_path).expect("archive");
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        writer.start_file("META/info.json", options).expect("meta");
        std::io::Write::write_all(&mut writer, br#"{"Name":"fixture"}"#).expect("meta body");
        writer
            .start_file("WAD/Ezreal.wad.client", options)
            .expect("wad");
        std::io::Write::write_all(&mut writer, b"RW\x03\x04").expect("wad body");
        writer.finish().expect("finish archive");
    }

    let target = root.join("81_81069");
    std::fs::create_dir_all(target.join("META")).expect("partial meta");
    assert!(
        !extracted_mod_is_complete(&target),
        "a directory with no WAD must not count as extracted"
    );

    prepare_mod_directory(&archive_path, &target).expect("re-extraction");

    assert!(
        extracted_mod_is_complete(&target),
        "the mod must be re-extracted rather than trusted for existing"
    );
    assert!(target.join("WAD").join("Ezreal.wad.client").is_file());

    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
}

fn decide(state: &dekan_core::state::AppState, current: Option<ArmKey>) -> ArmDecision {
    arm_decision(wanted_skin(state), current)
}

#[test]
fn test_arm_is_scheduled_for_a_valid_champ_select_selection() {
    let state = champ_select_state(Some(81), Some(target(81, 81065)));

    match decide(&state, None) {
        ArmDecision::Schedule(request) => {
            assert_eq!(request.key.champ_id, 81);
            assert_eq!(request.key.entry_id, Some(81065));
            assert_eq!(
                request.key.mods, 0,
                "no custom mod: the key is the same as without custom mod support"
            );
            assert_eq!(request.key.classic_slot, None);
            assert_eq!(request.key.party, 0);
            let now = tokio::time::Instant::now();
            assert!(
                request.due > now,
                "the rebuild must wait out the debounce window, not fire on the click"
            );
            let remaining = request.due.duration_since(now);
            assert!(
                remaining <= INITIAL_ARM_DEBOUNCE + Duration::from_millis(100),
                "initial debounce should be around 100ms, got {remaining:?}"
            );
        }
        ArmDecision::Settled => panic!("a valid selection must schedule an overlay build"),
    }

    let different_skin = ArmKey {
        champ_id: 81,
        entry_id: Some(81001),
        mods: 0,
        classic_slot: None,
        party: 0,
        second: None,
        lobby: false,
    };
    match decide(&state, Some(different_skin)) {
        ArmDecision::Schedule(request) => {
            let now = tokio::time::Instant::now();
            let remaining = request.due.duration_since(now);
            assert!(
                remaining > INITIAL_ARM_DEBOUNCE,
                "skin change debounce must be longer than initial debounce, got {remaining:?}"
            );
            assert!(
                remaining <= ARM_DEBOUNCE + Duration::from_millis(100),
                "skin change debounce should be around 900ms, got {remaining:?}"
            );
        }
        ArmDecision::Settled => panic!("a changed selection must schedule an overlay build"),
    }
}

#[test]
fn test_arm_is_settled_when_there_is_nothing_to_build() {
    assert!(matches!(
        decide(&champ_select_state(Some(81), None), None),
        ArmDecision::Settled
    ));

    assert!(matches!(
        decide(&champ_select_state(None, Some(target(81, 81065))), None),
        ArmDecision::Settled
    ));

    assert!(matches!(
        decide(&champ_select_state(Some(25), Some(target(81, 81065))), None),
        ArmDecision::Settled
    ));

    assert!(matches!(
        decide(&champ_select_state(Some(81), Some(target(81, 81000))), None),
        ArmDecision::Settled
    ));

    let same = ArmKey {
        champ_id: 81,
        entry_id: Some(81065),
        mods: 0,
        classic_slot: None,
        party: 0,
        second: None,
        lobby: false,
    };
    assert!(matches!(
        decide(
            &champ_select_state(Some(81), Some(target(81, 81065))),
            Some(same)
        ),
        ArmDecision::Settled
    ));
}

fn with_mods(mut state: dekan_core::state::AppState, map: &str) -> dekan_core::state::AppState {
    state.mods = dekan_core::mods::ModSelection {
        map: Some(map.into()),
        ..Default::default()
    };
    state
}

#[test]
fn test_custom_mods_alone_arm_a_build_without_a_skin() {
    let state = with_mods(champ_select_state(Some(81), None), "dekan:maps/Winter");
    match wanted_skin(&state) {
        WantedSkin::Skin(key) => {
            assert_eq!(key.entry_id, None, "no library skin to install");
            assert_ne!(key.mods, 0);
        }
        other => panic!("mods alone must still build an overlay, got {other:?}"),
    }

    let base = with_mods(
        champ_select_state(Some(81), Some(target(81, 81000))),
        "dekan:maps/Winter",
    );
    assert!(matches!(
        wanted_skin(&base),
        WantedSkin::Skin(ArmKey { entry_id: None, .. })
    ));
}

#[test]
fn test_changing_the_mods_changes_the_key_so_the_overlay_is_rebuilt() {
    let skin_only = champ_select_state(Some(81), Some(target(81, 81065)));
    let with_map = with_mods(skin_only.clone(), "dekan:maps/Winter");
    let (WantedSkin::Skin(a), WantedSkin::Skin(b)) =
        (wanted_skin(&skin_only), wanted_skin(&with_map))
    else {
        panic!("both selections build");
    };
    assert_eq!(a.entry_id, b.entry_id);
    assert_ne!(a, b, "adding a map mod must rebuild the armed overlay");
    assert!(should_disarm(wanted_skin(&with_map), a));
    assert!(matches!(
        arm_decision(wanted_skin(&with_map), Some(a)),
        ArmDecision::Schedule(_)
    ));
}

#[test]
fn test_an_unknown_champion_with_mods_is_silence_not_nothing() {
    let state = with_mods(champ_select_state(None, None), "dekan:maps/Winter");
    assert_eq!(wanted_skin(&state), WantedSkin::Unknown);
}

#[test]
fn test_rift_classic_ignores_mods_and_tracks_the_client_slot() {
    let mut state = with_mods(
        champ_select_state(Some(60_001), Some(target(60_001, 1005))),
        "dekan:maps/Winter",
    );
    state.selected_skin_id = Some(60_001_301);
    let WantedSkin::Skin(key) = wanted_skin(&state) else {
        panic!("a Classic skin must build");
    };
    assert_eq!(key.entry_id, Some(1005));
    assert_eq!(key.mods, 0, "custom mods do not apply to Rift Classic");
    assert_eq!(
        key.classic_slot, None,
        "301 is a default slot, already covered"
    );

    state.selected_skin_id = Some(60_001_007);
    let WantedSkin::Skin(moved) = wanted_skin(&state) else {
        panic!("still builds");
    };
    assert_eq!(moved.classic_slot, Some(7));
    assert!(
        should_disarm(wanted_skin(&state), key),
        "the client moving to another slot must rebuild the Classic mod"
    );

    let base = champ_select_state(Some(60_001), Some(target(60_001, 1000)));
    assert_eq!(wanted_skin(&base), WantedSkin::Nothing);
}

#[test]
fn test_outside_classic_the_client_skin_never_changes_the_key() {
    let mut state = champ_select_state(Some(81), Some(target(81, 81065)));
    let WantedSkin::Skin(before) = wanted_skin(&state) else {
        panic!("builds");
    };
    state.selected_skin_id = Some(81_000);
    assert_eq!(wanted_skin(&state), WantedSkin::Skin(before));
}

fn with_party(mut state: dekan_core::state::AppState) -> dekan_core::state::AppState {
    state.local_puuid = Some("me".into());
    state.team = vec![
        dekan_core::party::TeamMember {
            puuid: "me".into(),
            champion_id: 81,
        },
        dekan_core::party::TeamMember {
            puuid: "friend".into(),
            champion_id: 103,
        },
    ];
    state.party_peers = vec![dekan_core::party::PartyPeer {
        member_id: 9,
        puuid: "friend".into(),
        champion_id: 103,
        skin_id: 103_015,
        chroma_id: None,
    }];
    state
}

#[test]
fn test_a_verified_friend_alone_builds_and_a_spoof_does_not() {
    let state = with_party(champ_select_state(Some(81), None));
    let WantedSkin::Skin(key) = wanted_skin(&state) else {
        panic!("a teammate's skin is worth an overlay even without ours");
    };
    assert_eq!(key.entry_id, None);
    assert_ne!(key.party, 0);

    let mut spoofed = state.clone();
    spoofed.party_peers[0].champion_id = 1;
    spoofed.party_peers[0].skin_id = 1_005;
    assert_eq!(wanted_skin(&spoofed), WantedSkin::Nothing);

    let classic = with_party(champ_select_state(Some(60_081), None));
    assert_eq!(wanted_skin(&classic), WantedSkin::Nothing);
}

#[test]
fn test_a_patcher_for_an_abandoned_skin_is_disarmed_on_evidence_only() {
    let armed = ArmKey {
        champ_id: 81,
        entry_id: Some(81065),
        mods: 0,
        classic_slot: None,
        party: 0,
        second: None,
        lobby: false,
    };

    assert!(should_disarm(
        wanted_skin(&champ_select_state(Some(81), Some(target(81, 81069)))),
        armed
    ));

    assert!(should_disarm(
        wanted_skin(&champ_select_state(Some(81), Some(target(81, 81000)))),
        armed
    ));

    assert!(should_disarm(
        wanted_skin(&champ_select_state(Some(81), None)),
        armed
    ));

    assert!(should_disarm(
        wanted_skin(&champ_select_state(Some(25), Some(target(81, 81065)))),
        armed
    ));

    assert!(!should_disarm(
        wanted_skin(&champ_select_state(None, Some(target(81, 81065)))),
        armed
    ));

    assert!(!should_disarm(
        wanted_skin(&champ_select_state(Some(81), Some(target(81, 81065)))),
        armed
    ));
}

#[test]
fn test_resolved_paths_discovery_in_an_empty_profile() {
    let root = std::env::temp_dir().join(format!(
        "dekan_discover_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: fixture cleanup
    let data = root.join("Dekan");
    let state = data.join("state");

    let paths = ResolvedPaths::discover_in(state.clone(), data.clone());
    assert_eq!(paths.state_dir, state);
    assert_eq!(paths.overlay_dir, data.join("overlay"));
    assert_eq!(paths.mods_dir, data.join("mods"));
    for category in ["skins", "maps", "fonts", "ui", "others"] {
        assert!(
            data.join("custom_mods").join(category).is_dir(),
            "custom mods category {category} created"
        );
    }
    let _ = std::fs::remove_dir_all(&root); // ignore-ok: the fixture may not exist yet
}

fn tools_fixture(tag: &str, files: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dekan_tools_{tag}_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: the fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("fixture dir");
    for file in files {
        std::fs::write(dir.join(file), b"fixture").expect("fixture file");
    }
    dir
}

const BOTH_TOOLS: [&str; 2] = ["ltk_patcher_host.exe", "ltk_patcher_dll.dll"];

#[test]
fn test_first_candidate_folder_wins() {
    let first = tools_fixture("first", &BOTH_TOOLS);
    let second = tools_fixture("second", &BOTH_TOOLS);

    let (resolved, source) = resolve_tools_dir(&[first.clone(), second.clone()]);
    assert_eq!(resolved, first);
    assert_eq!(source, ToolsSource::Own);

    let _ = std::fs::remove_dir_all(&first); // ignore-ok: test temp dir teardown
    let _ = std::fs::remove_dir_all(&second); // ignore-ok: test temp dir teardown
}

#[test]
fn test_half_a_toolset_is_not_a_toolset() {
    let partial = tools_fixture("partial", &["ltk_patcher_host.exe"]);
    let (_, source) = resolve_tools_dir(std::slice::from_ref(&partial));
    assert_eq!(source, ToolsSource::Missing);
    let _ = std::fs::remove_dir_all(&partial); // ignore-ok: test temp dir teardown
}

#[test]
fn test_the_ltk_backend_is_the_toolset_and_another_injector_is_not() {
    let ltk = tools_fixture("ltk_only", &["ltk_patcher_host.exe", "ltk_patcher_dll.dll"]);
    let other = tools_fixture("other_only", &["other-injector.dll"]);
    assert_eq!(
        resolve_tools_dir(std::slice::from_ref(&ltk)).1,
        ToolsSource::Own
    );
    assert_eq!(
        resolve_tools_dir(std::slice::from_ref(&other)).1,
        ToolsSource::Missing
    );
    let _ = std::fs::remove_dir_all(&ltk); // ignore-ok: test temp dir teardown
    let _ = std::fs::remove_dir_all(&other); // ignore-ok: test temp dir teardown
}

#[test]
fn test_missing_tools_point_at_our_own_folder() {
    let absent = std::env::temp_dir().join("dekan_tools_absent_does_not_exist");
    let own = PathBuf::from(r"C:\Program Files\Dekan\tools");
    let (resolved, source) = resolve_tools_dir(&[own.clone(), absent]);

    if source == ToolsSource::Missing {
        assert_eq!(resolved, own);
    }
}

#[test]
fn test_game_dir_normalization_distinguishes_client_root() {
    let temp = std::env::temp_dir().join(format!("dekan_game_norm_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup

    let client_root = temp.join("League of Legends");
    std::fs::create_dir_all(client_root.join("DATA")).expect("client DATA");
    std::fs::write(client_root.join("LeagueClientUx.exe"), b"mock client").expect("client exe");

    let game_dir = client_root.join("Game");
    std::fs::create_dir_all(game_dir.join("DATA")).expect("game DATA");
    std::fs::write(game_dir.join("League of Legends.exe"), b"mock game").expect("game exe");

    let normalized = dekan_platform::paths::normalize_game_dir(&client_root);
    assert_eq!(normalized, Some(game_dir.clone()));

    let normalized_game = dekan_platform::paths::normalize_game_dir(&game_dir);
    assert_eq!(normalized_game, Some(game_dir));

    let _ = std::fs::remove_dir_all(&temp); // ignore-ok: cleanup
}

fn swiftplay_state(
    phase: dekan_core::phase::GamePhase,
    targets: Vec<dekan_core::overlay::OverlayTarget>,
) -> dekan_core::state::AppState {
    let slot = |champion_id: u32| dekan_core::lobby::LobbySlot {
        champion_id,
        skin_id: champion_id * 1000,
    };
    dekan_core::state::AppState {
        phase,
        champion_id: Some(238),
        lobby: Some(dekan_core::lobby::LobbyPicks {
            slots: vec![slot(238), slot(103)],
            targets,
        }),
        ..Default::default()
    }
}

#[test]
fn a_swiftplay_lobby_arms_one_patcher_for_both_champions() {
    use dekan_core::phase::GamePhase;
    let state = swiftplay_state(
        GamePhase::Matchmaking,
        vec![target(103, 103_015), target(238, 238_012)],
    );
    assert!(arms_in_lobby(&state));
    let WantedSkin::Skin(key) = wanted_skin(&state) else {
        panic!("both lobby skins must be armed");
    };
    assert!(key.lobby);
    assert_eq!(
        (key.champ_id, key.entry_id, key.second),
        (238, Some(238_012), Some((103, 103_015))),
        "slot order, not the order the skins were picked in"
    );
    assert!(key.covers(238) && key.covers(103) && !key.covers(1));
    assert_eq!(lobby_arm_key(&state), Some(key));
}

#[test]
fn a_lobby_with_base_skins_only_arms_nothing_and_the_match_itself_is_not_lobby_arming() {
    use dekan_core::phase::GamePhase;
    let base = swiftplay_state(GamePhase::Lobby, vec![target(238, 238_000)]);
    assert_eq!(wanted_skin(&base), WantedSkin::Nothing);

    let one = swiftplay_state(GamePhase::Lobby, vec![target(103, 103_015)]);
    let WantedSkin::Skin(key) = wanted_skin(&one) else {
        panic!("one lobby skin is enough to arm");
    };
    assert_eq!((key.champ_id, key.second), (103, None));

    assert!(!arms_in_lobby(&swiftplay_state(
        GamePhase::InProgress,
        Vec::new()
    )));
    assert!(!arms_in_lobby(&champ_select_state(Some(238), None)));
}

#[test]
fn a_lobby_left_over_in_champion_select_never_replaces_the_locked_champion() {
    use dekan_core::phase::GamePhase;
    let mut state = swiftplay_state(GamePhase::ChampSelect, vec![target(238, 238_012)]);
    state.champion_id = Some(84);
    state.overlay_target = Some(target(84, 84_009));
    let WantedSkin::Skin(key) = wanted_skin(&state) else {
        panic!("the champion select pick must be armed");
    };
    assert_eq!(
        (key.champ_id, key.entry_id, key.lobby),
        (84, Some(84_009), false)
    );
}

#[test]
fn a_lobby_key_knows_each_champions_skin_and_the_other_champions_mods() {
    use dekan_core::phase::GamePhase;
    let state = swiftplay_state(
        GamePhase::Lobby,
        vec![target(238, 238_012), target(103, 103_015)],
    );
    let key = lobby_arm_key(&state).expect("lobby key");
    assert_eq!(key.entry_for(238), Some(238_012));
    assert_eq!(key.entry_for(103), Some(103_015));
    assert_eq!(key.entry_for(1), None);
    assert_eq!(key.picks(), vec![(238, 238_012), (103, 103_015)]);

    let mut only_mods = swiftplay_state(GamePhase::Lobby, Vec::new());
    assert_eq!(lobby_arm_key(&only_mods), None);
    only_mods
        .mods
        .skin
        .insert(103, "dekan:skin/ahri-custom".into());
    let key = lobby_arm_key(&only_mods).expect("a custom skin of the second champion arms too");
    assert_eq!((key.entry_id, key.lobby), (None, true));
    assert_ne!(key.mods, 0);
}
