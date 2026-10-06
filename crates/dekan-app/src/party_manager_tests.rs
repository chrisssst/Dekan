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
