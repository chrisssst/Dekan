use super::*;

#[test]
fn default_state_is_idle() {
    let state = AppState::default();
    assert_eq!(state.phase, GamePhase::None);
    assert_eq!(state.injection, InjectionStatus::Idle);
    assert!(state.champion_id.is_none());
}

#[test]
fn entering_champ_select_resets_the_match() {
    let (tx, rx) = new_state_channel();

    set_champion(&tx, 1);
    set_selected_skin(&tx, 1001);

    set_phase(&tx, GamePhase::ChampSelect);

    let state = rx.borrow();
    assert_eq!(state.phase, GamePhase::ChampSelect);
    assert!(state.champion_id.is_none());
    assert!(state.selected_skin_id.is_none());
}

#[test]
fn overlay_target_survives_lcu_updates_and_dies_with_the_champ_select() {
    let (tx, rx) = new_state_channel();
    let target = OverlayTarget {
        champion_id: 238,
        skin_id: 238_001,
        chroma_id: Some(238_015),
    };
    set_overlay_target(&tx, target.clone());

    set_selected_skin(&tx, 238_000);
    set_champion(&tx, 238);
    assert_eq!(rx.borrow().overlay_target.as_ref(), Some(&target));

    clear_overlay_target(&tx);
    assert!(rx.borrow().overlay_target.is_none());

    set_overlay_target(&tx, target);
    set_phase(&tx, GamePhase::ChampSelect);
    assert!(
        rx.borrow().overlay_target.is_none(),
        "a new champ select starts with no target"
    );
}

#[test]
fn losing_the_client_drops_the_overlay_target() {
    let (tx, rx) = new_state_channel();
    set_overlay_target(
        &tx,
        OverlayTarget {
            champion_id: 103,
            skin_id: 103_045,
            chroma_id: None,
        },
    );
    clear_for_new_game(&tx);
    assert!(
        rx.borrow().overlay_target.is_none(),
        "a target cannot outlive the champ select it was made in"
    );
}

#[test]
fn base_skin_detection() {
    assert!(crate::selection::is_base_skin(1000, 1));
    assert!(crate::selection::is_base_skin(0, 1));

    assert!(!crate::selection::is_base_skin(1001, 1));
}

#[test]
fn a_new_champ_select_does_not_inherit_the_previous_match() {
    let (tx, rx) = new_state_channel();
    set_phase(&tx, GamePhase::Lobby);
    set_phase(&tx, GamePhase::ChampSelect);
    set_champion(&tx, 136);
    set_selected_skin(&tx, 136_011);
    set_team(
        &tx,
        Some("me".into()),
        vec![TeamMember {
            puuid: "me".into(),
            champion_id: 136,
        }],
    );
    set_overlay_target(
        &tx,
        OverlayTarget {
            champion_id: 136,
            skin_id: 136_038,
            chroma_id: None,
        },
    );
    set_mod_selection(
        &tx,
        ModSelection {
            skin: [(136, "dekan:skins/x".to_owned())].into_iter().collect(),
            ..ModSelection::default()
        },
    );

    set_phase(&tx, GamePhase::Finalization);
    assert_eq!(rx.borrow().champion_id, Some(136));

    for phase in [
        GamePhase::GameStart,
        GamePhase::InProgress,
        GamePhase::Reconnect,
        GamePhase::InProgress,
        GamePhase::WaitingForStats,
        GamePhase::PreEndOfGame,
        GamePhase::EndOfGame,
        GamePhase::None,
    ] {
        set_phase(&tx, phase);
    }
    assert_eq!(
        rx.borrow().champion_id,
        Some(136),
        "the game and its post-game keep the match"
    );

    set_phase(&tx, GamePhase::ChampSelect);
    let state = rx.borrow();
    assert!(state.champion_id.is_none());
    assert!(state.selected_skin_id.is_none());
    assert!(state.overlay_target.is_none());
    assert!(state.team.is_empty());
    assert!(
        state.local_puuid.is_none(),
        "our PUUID is re-read with the new roster"
    );
    assert!(
        !state.mods.skin.is_empty(),
        "the mod selection is not per match"
    );
}

fn finished_match(tx: &StateSender) {
    set_phase(tx, GamePhase::ChampSelect);
    set_champion(tx, 518);
    set_selected_skin(tx, 518_000);
    set_team(
        tx,
        Some("me".into()),
        vec![TeamMember {
            puuid: "me".into(),
            champion_id: 518,
        }],
    );
    set_overlay_target(
        tx,
        OverlayTarget {
            champion_id: 518,
            skin_id: 518_001,
            chroma_id: None,
        },
    );
    set_injection_status(tx, InjectionStatus::Confirmed);
    set_party_status(tx, PartyStatus::Connected { members: 2 });
    for phase in [
        GamePhase::GameStart,
        GamePhase::InProgress,
        GamePhase::EndOfGame,
    ] {
        set_phase(tx, phase);
    }
}

fn assert_no_match_state(state: &AppState, context: &str) {
    assert!(state.champion_id.is_none(), "{context}: champion");
    assert!(state.selected_skin_id.is_none(), "{context}: skin");
    assert!(state.overlay_target.is_none(), "{context}: target");
    assert!(state.team.is_empty(), "{context}: team");
    assert!(state.local_puuid.is_none(), "{context}: local PUUID");
    assert_eq!(
        state.injection,
        InjectionStatus::Idle,
        "{context}: injection"
    );
}

#[test]
fn the_previous_match_is_gone_before_the_next_ready_check() {
    for phase in [
        GamePhase::Lobby,
        GamePhase::Matchmaking,
        GamePhase::ReadyCheck,
    ] {
        let (tx, rx) = new_state_channel();
        finished_match(&tx);
        set_phase(&tx, phase);
        let state = rx.borrow();
        assert_eq!(state.phase, phase);
        assert_no_match_state(&state, &format!("{phase:?}"));
        assert_eq!(
            state.party_status,
            PartyStatus::Connected { members: 2 },
            "the party room is not per match"
        );
    }

    let (tx, rx) = new_state_channel();
    finished_match(&tx);
    set_phase(&tx, GamePhase::Matchmaking);
    set_phase(&tx, GamePhase::ReadyCheck);
    assert_no_match_state(&rx.borrow(), "ready check after the post-game");
}

#[test]
fn a_dodged_champ_select_leaves_nothing_for_the_next_one() {
    let (tx, rx) = new_state_channel();
    set_phase(&tx, GamePhase::ChampSelect);
    set_champion(&tx, 103);
    set_team(&tx, Some("me".into()), Vec::new());

    set_phase(&tx, GamePhase::Lobby);
    assert_no_match_state(&rx.borrow(), "lobby after a dodge");
}

#[test]
fn only_entering_a_pre_match_phase_starts_a_new_match() {
    use GamePhase as P;
    for (from, to) in [
        (P::EndOfGame, P::Lobby),
        (P::Lobby, P::Matchmaking),
        (P::Matchmaking, P::ReadyCheck),
        (P::ReadyCheck, P::Matchmaking),
        (P::ReadyCheck, P::ChampSelect),
        (P::None, P::ChampSelect),
    ] {
        assert!(starts_new_match(from, to), "{from:?} -> {to:?}");
    }
    for (from, to) in [
        (P::Lobby, P::Lobby),
        (P::ReadyCheck, P::ReadyCheck),
        (P::ChampSelect, P::ChampSelect),
        (P::ChampSelect, P::Finalization),
        (P::Finalization, P::GameStart),
        (P::InProgress, P::Reconnect),
        (P::WaitingForStats, P::EndOfGame),
        (P::EndOfGame, P::None),
    ] {
        assert!(!starts_new_match(from, to), "{from:?} -> {to:?}");
    }
}

#[test]
fn clear_for_new_game_resets_everything() {
    let (tx, rx) = new_state_channel();

    set_phase(&tx, GamePhase::ChampSelect);
    set_champion(&tx, 1);
    set_selected_skin(&tx, 1001);
    set_injection_status(&tx, InjectionStatus::Confirmed);

    clear_for_new_game(&tx);

    let state = rx.borrow();
    assert_eq!(state.phase, GamePhase::None);
    assert_eq!(state.injection, InjectionStatus::Idle);
    assert!(state.champion_id.is_none());
}

#[test]
fn hosting_a_party_is_a_named_transition_that_only_fires_on_change() {
    let (tx, rx) = new_state_channel();
    assert!(!rx.borrow().party_hosting);

    let mut watcher = rx.clone();
    watcher.mark_unchanged();
    set_party_hosting(&tx, true);
    assert!(watcher.has_changed().expect("sender alive"));
    assert!(rx.borrow().party_hosting);

    watcher.mark_unchanged();
    set_party_hosting(&tx, true);
    assert!(!watcher.has_changed().expect("sender alive"));

    set_party_hosting(&tx, false);
    assert!(!rx.borrow().party_hosting);
}

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
