use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{ClientWindowState, WindowRect, overlay_placement_on};

const CACHE_TTL: Duration = Duration::from_millis(800);
const DEFAULT_MACOS_CLIENT_RECT: WindowRect = WindowRect {
    left: 120,
    top: 120,
    right: 1400,
    bottom: 840,
};

static CACHED_STATE: Mutex<Option<(Instant, ClientWindowState)>> = Mutex::new(None);

#[must_use]
pub fn client_window_state() -> ClientWindowState {
    if let Ok(guard) = CACHED_STATE.lock()
        && let Some((at, state)) = *guard
        && at.elapsed() < CACHE_TTL
    {
        return state;
    }
    let running =
        crate::process::ProcessFinder::find_any_process(&crate::game_version::CLIENT_EXES)
            .ok()
            .flatten()
            .is_some();
    let state = if running {
        ClientWindowState::Visible(DEFAULT_MACOS_CLIENT_RECT)
    } else {
        ClientWindowState::Absent
    };
    if let Ok(mut guard) = CACHED_STATE.lock() {
        *guard = Some((Instant::now(), state));
    }
    state
}

pub(crate) fn take_foreground(_window: isize) {}

pub(crate) fn return_foreground_to_client() {}

#[must_use]
pub fn overlay_placement(client: WindowRect, width: i32, height: i32, padding: i32) -> WindowRect {
    overlay_placement_on(client, None, width, height, padding)
}
