use super::*;
use dekan_core::overlay::OverlayTarget;
use dekan_core::phase::GamePhase;

fn picking_103() -> AppState {
    AppState {
        phase: GamePhase::ChampSelect,
        champion_id: Some(103),
        local_puuid: Some("me-1".into()),
        overlay_target: Some(OverlayTarget {
            champion_id: 103,
            skin_id: 103_015,
            chroma_id: None,
        }),
        ..Default::default()
    }
}

#[test]
fn test_a_pick_is_announced_only_while_its_match_is_live() {
    let mut state = picking_103();
    for phase in [
        GamePhase::ChampSelect,
        GamePhase::Finalization,
        GamePhase::GameStart,
        GamePhase::InProgress,
        GamePhase::Reconnect,
    ] {
        state.phase = phase;
        assert!(
            announcement(&state, 7).is_some(),
            "{phase:?} belongs to the match the pick was made in"
        );
    }

    for phase in [
        GamePhase::None,
        GamePhase::Lobby,
        GamePhase::Matchmaking,
        GamePhase::ReadyCheck,
        GamePhase::CheckedIntoTournament,
        GamePhase::WaitingForStats,
        GamePhase::PreEndOfGame,
        GamePhase::EndOfGame,
        GamePhase::FailedToLaunch,
        GamePhase::TerminatedInError,
    ] {
        state.phase = phase;
        assert_eq!(
            announcement(&state, 7),
            None,
            "{phase:?}: a champion here can only be a stale one"
        );
    }
}

#[test]
fn test_we_announce_only_a_valid_pick_for_our_own_champion() {
    let mut state = AppState {
        phase: GamePhase::ChampSelect,
        champion_id: Some(103),
        local_puuid: Some("me-1".into()),
        overlay_target: Some(OverlayTarget {
            champion_id: 103,
            skin_id: 103_015,
            chroma_id: None,
        }),
        ..Default::default()
    };
    let a = announcement(&state, 7).expect("announces");
    assert_eq!((a.member_id, a.champion_id, a.skin_id), (7, 103, 103_015));

    state.champion_id = Some(1);
    assert_eq!(
        announcement(&state, 7),
        None,
        "a stale pick for another champion is not sent"
    );

    state.champion_id = Some(60_103);
    state.overlay_target = Some(OverlayTarget {
        champion_id: 60_103,
        skin_id: 103_015,
        chroma_id: None,
    });
    assert_eq!(
        announcement(&state, 7),
        None,
        "Rift Classic is not part of party"
    );

    state.champion_id = Some(103);
    state.overlay_target = Some(OverlayTarget {
        champion_id: 103,
        skin_id: 103_015,
        chroma_id: None,
    });
    state.local_puuid = None;
    assert_eq!(
        announcement(&state, 7),
        None,
        "without our PUUID nobody could verify us"
    );
}

#[test]
fn test_a_stable_session_restarts_the_reconnect_backoff() {
    assert_eq!(retry_backoff(BACKOFF_MAX, STABLE_SESSION), BACKOFF_MIN);
    assert_eq!(
        retry_backoff(BACKOFF_MAX, Duration::from_secs(3 * 3600)),
        BACKOFF_MIN
    );
    assert_eq!(
        retry_backoff(Duration::from_secs(8), Duration::from_secs(2)),
        Duration::from_secs(8)
    );
}
