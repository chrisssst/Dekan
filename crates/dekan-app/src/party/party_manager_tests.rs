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
