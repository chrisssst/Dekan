use super::*;

fn lobby_slot(champion_id: ChampionId, skin_id: SkinId) -> LobbySlot {
    LobbySlot {
        champion_id,
        skin_id,
    }
}

fn pick(champion_id: ChampionId, skin_id: SkinId) -> OverlayTarget {
    OverlayTarget {
        champion_id,
        skin_id,
        chroma_id: None,
    }
}

#[test]
fn a_swiftplay_lobby_focuses_its_first_champion_and_keeps_a_skin_per_champion() {
    let (tx, rx) = new_state_channel();
    set_phase(&tx, GamePhase::Lobby);
    assert!(set_lobby_picks(
        &tx,
        Some(vec![lobby_slot(238, 238_000), lobby_slot(103, 103_007)])
    ));
    assert_eq!(rx.borrow().champion_id, Some(238));
    assert_eq!(rx.borrow().selected_skin_id, Some(238_000));

    set_overlay_target(&tx, pick(238, 238_012));
    assert!(focus_lobby_champion(&tx, 103));
    {
        let state = rx.borrow();
        assert_eq!(state.champion_id, Some(103));
        assert_eq!(state.selected_skin_id, Some(103_007));
        assert!(state.overlay_target.is_none());
    }
    set_overlay_target(&tx, pick(103, 103_015));

    assert!(focus_lobby_champion(&tx, 238));
    assert_eq!(
        rx.borrow().overlay_target,
        Some(pick(238, 238_012)),
        "going back to a champion brings its skin back"
    );
    assert!(
        !focus_lobby_champion(&tx, 1),
        "a champion outside the lobby"
    );

    let lobby = rx.borrow().lobby.clone().expect("lobby");
    assert_eq!(
        lobby.chosen_in_slot_order(),
        vec![&pick(238, 238_012), &pick(103, 103_015)]
    );
}

#[test]
fn queueing_keeps_the_lobby_picks_until_the_match_and_leaving_the_queue_drops_them() {
    let (tx, rx) = new_state_channel();
    set_phase(&tx, GamePhase::Lobby);
    set_lobby_picks(&tx, Some(vec![lobby_slot(238, 238_000)]));
    set_overlay_target(&tx, pick(238, 238_012));

    for phase in [
        GamePhase::Matchmaking,
        GamePhase::ReadyCheck,
        GamePhase::Matchmaking,
        GamePhase::ChampSelect,
    ] {
        set_phase(&tx, phase);
        assert_eq!(
            rx.borrow().overlay_target,
            Some(pick(238, 238_012)),
            "{phase:?} must not wipe the skin picked in the lobby"
        );
    }

    set_phase(&tx, GamePhase::Lobby);
    set_lobby_picks(&tx, None);
    let state = rx.borrow();
    assert!(state.lobby.is_none());
    assert!(state.champion_id.is_none());
    assert!(state.overlay_target.is_none());
}

#[test]
fn back_in_the_lobby_after_a_match_the_picks_come_back() {
    let (tx, rx) = new_state_channel();
    set_phase(&tx, GamePhase::Lobby);
    set_lobby_picks(&tx, Some(vec![lobby_slot(238, 238_000)]));
    set_overlay_target(&tx, pick(238, 238_012));
    set_phase(&tx, GamePhase::InProgress);
    set_phase(&tx, GamePhase::EndOfGame);
    set_phase(&tx, GamePhase::Lobby);

    let state = rx.borrow();
    assert_eq!(state.champion_id, Some(238));
    assert_eq!(state.overlay_target, Some(pick(238, 238_012)));
}

#[test]
fn a_normal_queue_has_no_lobby_picks_and_resets_as_before() {
    let (tx, rx) = new_state_channel();
    set_phase(&tx, GamePhase::Lobby);
    assert!(!set_lobby_picks(&tx, None));
    set_champion(&tx, 238);
    set_phase(&tx, GamePhase::Matchmaking);
    assert!(rx.borrow().champion_id.is_none());
}

#[test]
fn the_queue_is_published_only_when_it_changes() {
    let (tx, _rx) = new_state_channel();
    let aram = QueueInfo {
        queue_id: 450,
        game_mode: "ARAM".into(),
        map_id: 12,
    };
    assert!(set_queue(&tx, Some(aram.clone())));
    assert!(!set_queue(&tx, Some(aram)));
    assert!(set_queue(&tx, None));
}

#[test]
fn the_lobby_survives_its_delete_event_once_the_match_is_under_way() {
    let (tx, rx) = new_state_channel();
    set_phase(&tx, GamePhase::Lobby);
    set_lobby_picks(
        &tx,
        Some(vec![lobby_slot(238, 238_000), lobby_slot(103, 103_000)]),
    );
    set_overlay_target(&tx, pick(103, 103_015));
    set_phase(&tx, GamePhase::GameStart);

    assert!(!set_lobby_picks(&tx, None));
    assert_eq!(
        rx.borrow()
            .lobby
            .as_ref()
            .and_then(|lobby| lobby.target_for(103).cloned()),
        Some(pick(103, 103_015)),
        "the game start must still see the skins picked in the lobby"
    );

    set_phase(&tx, GamePhase::EndOfGame);
    assert!(set_lobby_picks(&tx, None));
    assert!(rx.borrow().lobby.is_none());
}

#[test]
fn a_lobby_target_restored_for_the_other_champion_can_be_taken_back() {
    let (tx, rx) = new_state_channel();
    set_phase(&tx, GamePhase::Lobby);
    set_lobby_picks(
        &tx,
        Some(vec![lobby_slot(238, 238_000), lobby_slot(103, 103_000)]),
    );
    assert!(set_lobby_target(&tx, &pick(103, 103_015)));
    assert!(
        rx.borrow().overlay_target.is_none(),
        "the focused champion keeps its own pick"
    );
    assert!(!set_lobby_target(&tx, &pick(1, 1_001)), "not in the lobby");

    assert!(clear_lobby_target(&tx, 103));
    assert!(!clear_lobby_target(&tx, 103));
    assert!(
        rx.borrow()
            .lobby
            .as_ref()
            .is_some_and(|l| l.targets.is_empty())
    );
}
