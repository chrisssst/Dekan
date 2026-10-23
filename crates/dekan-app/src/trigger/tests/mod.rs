use super::arm_key::{WantedSkin, wanted_skin, wanted_skin_when};
use super::*;

pub(super) fn champ_select_state(
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

pub(super) fn target(champion_id: u32, skin_id: u32) -> dekan_core::overlay::OverlayTarget {
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

fn decide(state: &dekan_core::state::AppState, current: Option<ArmKey>) -> ArmDecision {
    arm_decision(wanted_skin(state), current)
}

#[test]
fn test_on_the_reconnect_screen_only_a_new_pick_rebuilds() {
    let mut state = champ_select_state(Some(81), Some(target(81, 81065)));
    state.phase = GamePhase::Reconnect;
    let WantedSkin::Skin(kept) = wanted_skin(&state) else {
        panic!("the skin of the match is still wanted on the reconnect screen");
    };
    assert!(matches!(decide(&state, Some(kept)), ArmDecision::Settled));

    state.overlay_target = Some(target(81, 81001));
    assert!(matches!(
        decide(&state, Some(kept)),
        ArmDecision::Schedule(_)
    ));

    state.overlay_target = None;
    assert!(should_disarm(wanted_skin(&state), kept));
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
fn test_a_teammate_pick_change_keeps_the_armed_patcher_until_its_replacement() {
    let armed = ArmKey {
        champ_id: 9,
        entry_id: Some(9034),
        mods: 0,
        classic_slot: None,
        party: 2_284_058_962_661_815_573,
        second: None,
        lobby: false,
    };
    let teammate_changed = ArmKey {
        party: 7_850_864_699_615_393_803,
        ..armed
    };

    assert!(armed.same_own_pick(&teammate_changed));
    assert!(!should_disarm(WantedSkin::Skin(teammate_changed), armed));
    assert!(matches!(
        arm_decision(WantedSkin::Skin(teammate_changed), Some(armed)),
        ArmDecision::Schedule(request) if request.key == teammate_changed
    ));

    let own_changed = ArmKey {
        entry_id: Some(9027),
        ..teammate_changed
    };
    assert!(!armed.same_own_pick(&own_changed));
    assert!(should_disarm(WantedSkin::Skin(own_changed), armed));
}

mod extract;
mod files;
mod lobby;

#[test]
fn the_reconnect_screen_keeps_nothing_armed_without_a_running_patcher() {
    let state = champ_select_state(Some(238), Some(target(238, 238_001)));
    for wanted in [
        WantedSkin::Unknown,
        WantedSkin::Nothing,
        wanted_skin(&state),
    ] {
        let (armed, running) = resume_for_reconnect(None, wanted);
        assert!(armed.is_none() && running.is_none());
    }
}
