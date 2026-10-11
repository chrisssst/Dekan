use super::arm_key::{WantedSkin, lobby_arm_key};
use super::*;
use super::{champ_select_state, target};

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
