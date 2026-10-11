use super::*;

#[must_use]
pub fn decide_placement(
    state: ClientWindowState,
    wanted: bool,
    monitor: Option<WindowRect>,
) -> Option<WindowRect> {
    if !wanted {
        return None;
    }
    match state {
        ClientWindowState::Visible(rect) => {
            let (width, height) = overlay_size();
            Some(overlay_placement_on(
                rect,
                monitor,
                width,
                height,
                OVERLAY_PADDING,
            ))
        }
        ClientWindowState::Hidden | ClientWindowState::Absent => None,
    }
}

#[derive(Debug, Default)]
pub struct OverlayTracker {
    last_client_rect: Option<WindowRect>,
    was_wanted: bool,
}

impl OverlayTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(&mut self, controller: &OverlayController, wanted: bool) -> Option<WindowRect> {
        if !wanted {
            if self.was_wanted {
                controller.hide();
                self.was_wanted = false;
                self.last_client_rect = None;
            }
            return None;
        }

        match client_window_state() {
            ClientWindowState::Visible(client_rect) => {
                let (width, height) = overlay_size();
                let rect = overlay_placement(client_rect, width, height, OVERLAY_PADDING);

                if self.last_client_rect != Some(client_rect) {
                    controller.show_at(rect);
                    self.last_client_rect = Some(client_rect);
                }
                self.was_wanted = true;
                Some(rect)
            }
            ClientWindowState::Hidden | ClientWindowState::Absent => {
                if self.was_wanted {
                    controller.hide();
                    self.was_wanted = false;
                    self.last_client_rect = None;
                }
                None
            }
        }
    }
}

static GLOBAL_TRACKER: Mutex<Option<OverlayTracker>> = Mutex::new(None);

pub fn track_once(controller: &OverlayController, wanted: bool) -> Option<WindowRect> {
    let mut lock = GLOBAL_TRACKER.lock().unwrap_or_else(|e| e.into_inner());
    let tracker = lock.get_or_insert_with(OverlayTracker::new);
    tracker.tick(controller, wanted)
}
