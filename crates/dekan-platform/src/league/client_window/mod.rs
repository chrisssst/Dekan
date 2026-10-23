#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{client_window_state, overlay_placement};
#[cfg(target_os = "macos")]
pub(crate) use macos::{return_foreground_to_client, take_foreground};
#[cfg(windows)]
pub use windows::{
    LEAGUE_CLIENT_CLASS, client_window_state, find_client_hwnd, get_monitor_work_area,
    overlay_placement,
};
#[cfg(windows)]
pub(crate) use windows::{return_foreground_to_client, take_foreground};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl WindowRect {
    #[must_use]
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    #[must_use]
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    #[must_use]
    pub fn is_on_screen(&self) -> bool {
        self.left > -30000 && self.top > -30000 && self.width() > 0 && self.height() > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientWindowState {
    Absent,

    Hidden,

    Visible(WindowRect),
}

#[must_use]
pub fn overlay_placement_with_monitor(
    client: WindowRect,
    monitor: WindowRect,
    width: i32,
    height: i32,
    padding: i32,
) -> WindowRect {
    let height = height.min(monitor.height() - padding * 2).max(120);

    let left = if client.right + padding + width <= monitor.right {
        client.right + padding
    } else if client.left - padding - width >= monitor.left {
        client.left - padding - width
    } else {
        (monitor.right - width - padding).max(monitor.left + padding)
    };

    let mut top = client.top + (client.height() - height) / 2;
    if top + height > monitor.bottom - padding {
        top = (monitor.bottom - padding - height).max(monitor.top + padding);
    }
    if top < monitor.top + padding {
        top = monitor.top + padding;
    }

    WindowRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

#[must_use]
pub fn overlay_placement_on(
    client: WindowRect,
    monitor: Option<WindowRect>,
    width: i32,
    height: i32,
    padding: i32,
) -> WindowRect {
    if let Some(monitor) = monitor {
        return overlay_placement_with_monitor(client, monitor, width, height, padding);
    }

    let height = height.min(client.height() - padding * 2).max(120);
    let left = client.right + padding;
    let top = client.top + (client.height() - height) / 2;
    WindowRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> WindowRect {
        WindowRect {
            left: 100,
            top: 50,
            right: 1380,
            bottom: 770,
        }
    }

    #[test]
    fn test_minimized_rect_is_not_on_screen() {
        let minimized = WindowRect {
            left: -32000,
            top: -32000,
            right: -31840,
            bottom: -31972,
        };
        assert!(
            !minimized.is_on_screen(),
            "the minimized sentinel must never be treated as a position"
        );
        assert!(client().is_on_screen());
    }

    #[test]
    fn test_overlay_placement_places_outside_when_monitor_has_space() {
        let client = WindowRect {
            left: 100,
            top: 100,
            right: 1380,
            bottom: 820,
        };
        let monitor = WindowRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let placement = overlay_placement_with_monitor(client, monitor, 360, 520, 16);

        assert_eq!(placement.width(), 360);

        assert_eq!(placement.left, client.right + 16);
        assert!(placement.right <= monitor.right);
        assert!(placement.top >= monitor.top);
        assert!(placement.bottom <= monitor.bottom);
    }

    #[test]
    fn test_overlay_placement_falls_back_to_left_when_right_has_no_space() {
        let client = WindowRect {
            left: 500,
            top: 100,
            right: 1780,
            bottom: 820,
        };
        let monitor = WindowRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let placement = overlay_placement_with_monitor(client, monitor, 360, 520, 16);

        assert_eq!(placement.left, client.left - 16 - 360);
        assert!(placement.left >= monitor.left);
    }

    #[test]
    fn test_overlay_shrinks_for_a_short_client_window() {
        let short = WindowRect {
            left: 0,
            top: 0,
            right: 800,
            bottom: 300,
        };
        let placement = overlay_placement_on(short, None, 360, 520, 16);

        assert!(
            placement.height() <= short.height(),
            "the overlay must not spill outside a small client window"
        );
        assert!(placement.top >= short.top);
    }

    #[test]
    fn test_client_window_state_is_reported_without_panicking() {
        let state = client_window_state();
        match state {
            ClientWindowState::Visible(rect) => assert!(rect.is_on_screen()),
            ClientWindowState::Hidden | ClientWindowState::Absent => {}
        }
    }
}
