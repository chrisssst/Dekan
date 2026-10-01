use super::*;

fn client_rect() -> WindowRect {
    WindowRect {
        left: 0,
        top: 0,
        right: 1600,
        bottom: 900,
    }
}

#[test]
fn test_packing_survives_negative_coordinates() {
    for (x, y) in [(0, 0), (100, 50), (-1920, -200), (3000, 1400)] {
        assert_eq!(unpack_point(pack_point(x, y)), (x, y), "roundtrip {x},{y}");
    }
}

#[test]
fn test_hidden_client_hides_the_overlay() {
    assert!(decide_placement(ClientWindowState::Hidden, true, None).is_none());
    assert!(decide_placement(ClientWindowState::Absent, true, None).is_none());
}

#[test]
fn test_overlay_is_not_shown_when_not_wanted() {
    assert!(
        decide_placement(ClientWindowState::Visible(client_rect()), false, None).is_none(),
        "outside champ select the overlay must stay hidden even with the client on screen"
    );
}

#[test]
fn test_visible_client_places_the_overlay_alongside_it() {
    let placement = decide_placement(ClientWindowState::Visible(client_rect()), true, None)
        .expect("overlay should be placed");
    assert_eq!(placement.width(), OVERLAY_WIDTH);
    assert!(placement.left >= client_rect().right);
}
