use super::*;

#[test]
fn test_clicks_queued_behind_the_join_window_are_ignored() {
    let clicks = sort_queued_clicks([
        PartyCommand::Create,
        PartyCommand::Create,
        PartyCommand::Join,
    ]);
    assert_eq!(
        clicks,
        QueuedClicks {
            ignored: 3,
            leave: false
        }
    );
}

#[test]
fn test_a_queued_leave_still_leaves_the_room() {
    let clicks = sort_queued_clicks([PartyCommand::Create, PartyCommand::Leave]);
    assert!(clicks.leave);
    assert_eq!(clicks.ignored, 1);
}

#[test]
fn test_an_empty_queue_changes_nothing() {
    assert_eq!(sort_queued_clicks([]), QueuedClicks::default());
}

#[test]
fn a_room_this_dekan_created_is_recognised_by_its_code_and_others_are_not() {
    let hosted = HostedRooms::default();
    let mine = PartyToken::generate(7, unix_now()).expect("token");
    let friend = PartyToken::generate(8, unix_now()).expect("token");
    hosted.remember(&mine);
    assert!(hosted.contains(&mine));
    assert!(hosted.is_own_code(&mine.encode()));
    assert!(
        hosted.is_own_code(&format!("  {}  ", mine.encode())),
        "pasted with spaces"
    );
    assert!(!hosted.contains(&friend));
    assert!(!hosted.is_own_code(&friend.encode()));
    assert!(!hosted.is_own_code("DEKAN1:garbage"));
    assert!(
        !HostedRooms::default().is_own_code(&mine.encode()),
        "a fresh session hosts nothing"
    );
}

#[test]
fn test_a_new_lobby_switches_the_room_and_a_left_one_closes_it() {
    use lobby::{LobbyStep, lobby_step};
    assert_eq!(
        lobby_step(None, Some("party-a"), 0, false, true, false),
        LobbyStep::Switch
    );
    assert_eq!(
        lobby_step(Some("party-a"), Some("party-b"), 3, true, true, false),
        LobbyStep::Switch
    );
    assert_eq!(
        lobby_step(Some("party-a"), None, 3, true, true, false),
        LobbyStep::Switch
    );
    assert_eq!(
        lobby_step(None, None, 0, false, true, false),
        LobbyStep::Stay
    );
}

#[test]
fn test_the_invite_waits_for_a_friend_the_lobby_and_no_earlier_answer() {
    use lobby::{LobbyStep, lobby_step};
    let here = Some("party-a");
    assert_eq!(
        lobby_step(here, here, 3, false, true, false),
        LobbyStep::Ask { friends: 2 }
    );
    assert_eq!(
        lobby_step(here, here, 1, false, true, false),
        LobbyStep::Stay
    );
    assert_eq!(
        lobby_step(here, here, 3, true, true, false),
        LobbyStep::Stay
    );
    assert_eq!(
        lobby_step(here, here, 3, false, false, false),
        LobbyStep::Stay
    );
    assert_eq!(
        lobby_step(here, here, 3, false, true, true),
        LobbyStep::Stay
    );
}

#[test]
fn test_an_open_invite_closes_when_the_lobby_or_the_phase_moves_on() {
    use lobby::invite_outdated;
    let here = Some("party-a");
    assert!(!invite_outdated(None, None, false));
    assert!(!invite_outdated(here, here, true));
    assert!(invite_outdated(here, here, false));
    assert!(invite_outdated(here, Some("party-b"), true));
    assert!(invite_outdated(here, None, true));
}
