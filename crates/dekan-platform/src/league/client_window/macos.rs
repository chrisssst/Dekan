use super::{ClientWindowState, WindowRect, overlay_placement_on};

#[must_use]
pub fn client_window_state() -> ClientWindowState {
    ClientWindowState::Absent
}

pub(crate) fn take_foreground(_window: isize) {}

pub(crate) fn return_foreground_to_client() {}

#[must_use]
pub fn overlay_placement(client: WindowRect, width: i32, height: i32, padding: i32) -> WindowRect {
    overlay_placement_on(client, None, width, height, padding)
}
