use super::*;

fn target(champion_id: ChampionId) -> OverlayTarget {
    OverlayTarget {
        champion_id,
        skin_id: champion_id * 1000 + 1,
        chroma_id: None,
    }
}

#[test]
fn a_pick_for_another_champion_is_stale() {
    assert!(
        target_is_stale(Some(&target(238)), Some(103)),
        "a Zed pick must not survive a swap to Ahri"
    );
}

#[test]
fn a_quiet_client_does_not_invalidate_the_pick() {
    assert!(
        !target_is_stale(Some(&target(238)), None),
        "the champion is unknown between phases; the pick has to survive that"
    );
    assert!(!target_is_stale(Some(&target(238)), Some(238)));
    assert!(!target_is_stale(None, Some(238)));
}

fn in_phase(phase: GamePhase, lobby: bool) -> AppState {
    AppState {
        phase,
        lobby: lobby.then(dekan_core::lobby::LobbyPicks::default),
        ..AppState::default()
    }
}

#[test]
fn the_overlay_shows_only_during_champ_select() {
    assert!(wanted_for_state(&in_phase(GamePhase::ChampSelect, false)));
    assert!(wanted_for_state(&in_phase(GamePhase::Finalization, false)));
    for phase in [
        GamePhase::None,
        GamePhase::Lobby,
        GamePhase::Matchmaking,
        GamePhase::ReadyCheck,
        GamePhase::GameStart,
        GamePhase::InProgress,
        GamePhase::EndOfGame,
    ] {
        assert!(
            !wanted_for_state(&in_phase(phase, false)),
            "the overlay must stay off screen in {phase:?} — never over the game"
        );
    }
}

#[test]
fn a_lobby_that_picks_champions_shows_the_overlay_until_the_match_starts() {
    for phase in [
        GamePhase::Lobby,
        GamePhase::Matchmaking,
        GamePhase::ReadyCheck,
    ] {
        assert!(wanted_for_state(&in_phase(phase, true)), "{phase:?}");
    }
    for phase in [
        GamePhase::GameStart,
        GamePhase::InProgress,
        GamePhase::EndOfGame,
    ] {
        assert!(
            !wanted_for_state(&in_phase(phase, true)),
            "never over the game: {phase:?}"
        );
    }
    assert!(!last_call_for_a_skin(&in_phase(GamePhase::Lobby, true)));
    assert!(last_call_for_a_skin(&in_phase(
        GamePhase::Matchmaking,
        true
    )));
    assert!(!last_call_for_a_skin(&in_phase(
        GamePhase::Matchmaking,
        false
    )));
}

fn zed_catalog() -> Catalog {
    use crate::catalog::{CatalogChroma, CatalogSkin};
    Catalog {
        champion_id: 238,
        champion_name: "Zed".into(),
        alias: Some("Zed".into()),
        skins: vec![CatalogSkin {
            id: 238_001,
            name: "Shockblade Zed".into(),
            name_unknown: false,
            chromas: vec![CatalogChroma {
                id: 238_004,
                name: "Shockblade Zed (Ruby)".into(),
                color: None,
                form: false,
                preview_path: Some(
                    "/lol-game-data/assets/v1/champion-chroma-images/238/238004.png".into(),
                ),
                has_preview: true,
            }],
            tile: None,
        }],
        locale: None,
        quote: None,
        mods: Default::default(),
        notice: None,
        classic: false,
        lobby: Vec::new(),
    }
}

#[test]
fn random_mode_rolls_a_skin_or_one_of_its_chromas_never_the_base() {
    use crate::catalog::CatalogSkin;
    let mut catalog = zed_catalog();
    catalog.skins.insert(
        0,
        CatalogSkin {
            id: 238_000,
            name: "Zed".into(),
            name_unknown: false,
            chromas: vec![],
            tile: None,
        },
    );

    let skin = catalog.roll_random(|_| 0).expect("roll");
    assert_eq!((skin.skin_id, skin.chroma_id), (238_001, None));
    let mut picks = [0usize, 1].into_iter();
    let chroma = catalog
        .roll_random(|_| picks.next().unwrap_or(0))
        .expect("roll");
    assert_eq!((chroma.skin_id, chroma.chroma_id), (238_001, Some(238_004)));

    for n in 0..50 {
        let target = catalog.roll_random(|_| n).expect("roll");
        assert_ne!(target.skin_id, 238_000, "the base skin is never rolled");
    }

    catalog.skins.retain(|s| s.id == 238_000);
    assert_eq!(catalog.roll_random(|_| 0), None);
    assert!(random_index(3) < 3);
}

#[test]
fn a_clicked_skin_becomes_the_published_target() {
    let (tx, rx) = dekan_core::state::new_state_channel();
    let catalog = zed_catalog();

    handle_command(&tx, OverlayCommand::Select { id: 238_001 }, Some(&catalog));

    let published = rx
        .borrow()
        .overlay_target
        .clone()
        .expect("the click must publish a target");
    assert_eq!(published.champion_id, 238);
    assert_eq!(published.skin_id, 238_001);
    assert_eq!(published.chroma_id, None);
}

#[test]
fn a_clicked_chroma_publishes_the_chroma_as_the_package() {
    let (tx, rx) = dekan_core::state::new_state_channel();
    let catalog = zed_catalog();

    handle_command(&tx, OverlayCommand::Select { id: 238_004 }, Some(&catalog));

    let published = rx
        .borrow()
        .overlay_target
        .clone()
        .expect("the chroma click must publish a target");
    assert_eq!(published.skin_id, 238_001, "the parent skin travels along");
    assert_eq!(published.chroma_id, Some(238_004));
    assert_eq!(
        published.package_entry_id(),
        238_004,
        "the chroma package is what gets installed"
    );
}

#[test]
fn an_id_outside_the_catalog_publishes_nothing() {
    let (tx, rx) = dekan_core::state::new_state_channel();
    let catalog = zed_catalog();

    handle_command(&tx, OverlayCommand::Select { id: 999_999 }, Some(&catalog));
    assert!(
        rx.borrow().overlay_target.is_none(),
        "refusing to guess means publishing nothing at all"
    );
}

#[test]
fn a_selection_without_a_catalog_publishes_nothing() {
    let (tx, rx) = dekan_core::state::new_state_channel();

    handle_command(&tx, OverlayCommand::Select { id: 238_001 }, None);
    assert!(rx.borrow().overlay_target.is_none());
}

#[test]
fn clearing_removes_the_target() {
    let (tx, rx) = dekan_core::state::new_state_channel();
    let catalog = zed_catalog();

    handle_command(&tx, OverlayCommand::Select { id: 238_001 }, Some(&catalog));
    handle_command(&tx, OverlayCommand::Clear, Some(&catalog));
    assert!(rx.borrow().overlay_target.is_none());
}

fn fallback() -> RandomFallback {
    RandomFallback {
        enabled: true,
        finalization: true,
        target_chosen: false,
        already_rolled: false,
        declined: false,
        lcu_skin: Some(157_000),
    }
}

#[test]
fn test_a_locked_champion_without_a_skin_gets_a_random_one() {
    assert!(should_roll_random(157, fallback()));
    assert!(should_roll_random(
        157,
        RandomFallback {
            lcu_skin: None,
            ..fallback()
        }
    ));
}

#[test]
fn test_the_random_skin_never_overrides_a_choice() {
    for blocked in [
        RandomFallback {
            enabled: false,
            ..fallback()
        },
        RandomFallback {
            finalization: false,
            ..fallback()
        },
        RandomFallback {
            target_chosen: true,
            ..fallback()
        },
        RandomFallback {
            already_rolled: true,
            ..fallback()
        },
        RandomFallback {
            declined: true,
            ..fallback()
        },
        RandomFallback {
            lcu_skin: Some(157_005),
            ..fallback()
        },
    ] {
        assert!(!should_roll_random(157, blocked), "{blocked:?}");
    }
}
