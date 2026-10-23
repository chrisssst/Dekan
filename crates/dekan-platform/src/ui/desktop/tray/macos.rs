use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::debug;

use super::TrayEvent;
use crate::error::PlatformError;

#[derive(Debug, Clone, Default)]
struct PartyMenu {
    line: String,
    in_room: bool,
}

#[derive(Clone)]
pub struct TrayController {
    alive: Arc<AtomicBool>,
    status: Arc<Mutex<String>>,
    party: Arc<Mutex<PartyMenu>>,
    events: UnboundedSender<TrayEvent>,
}

impl TrayController {
    pub fn update_status(&self, status: &str) {
        if self.alive.load(Ordering::Relaxed) {
            *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status.to_string();
        }
    }

    #[must_use]
    pub fn status(&self) -> String {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    #[must_use]
    pub fn party(&self) -> (String, bool) {
        let party = self.party.lock().unwrap_or_else(|e| e.into_inner());
        (party.line.clone(), party.in_room)
    }

    #[must_use]
    pub fn events(&self) -> UnboundedSender<TrayEvent> {
        self.events.clone()
    }

    pub fn update_party(&self, line: &str, in_room: bool) {
        let mut party = self.party.lock().unwrap_or_else(|e| e.into_inner());
        party.line = line.to_string();
        party.in_room = in_room;
    }

    pub fn notify(&self, title: &str, body: &str) {
        if !self.alive.load(Ordering::Relaxed) {
            return;
        }
        let (title, body) = (title.to_owned(), body.to_owned());
        let spawned = std::thread::Builder::new()
            .name("dekan-notify".into())
            .spawn(move || {
                if !crate::shell::notify(&title, &body) {
                    debug!(title, "The macOS notification was not shown");
                }
            });
        if let Err(e) = spawned {
            debug!(error = %e, "The macOS notification thread could not start");
        }
    }

    pub fn shutdown(&self) {
        self.alive.store(false, Ordering::SeqCst);
    }
}

pub struct SystemTray {
    event_rx: UnboundedReceiver<TrayEvent>,
    controller: TrayController,
}

impl SystemTray {
    pub fn spawn(initial_title: &str) -> Result<Self, PlatformError> {
        let (event_tx, event_rx) = unbounded_channel();
        Ok(Self {
            event_rx,
            controller: TrayController {
                alive: Arc::new(AtomicBool::new(true)),
                status: Arc::new(Mutex::new(initial_title.to_string())),
                party: Arc::new(Mutex::new(PartyMenu {
                    line: crate::i18n::text().party_off.into(),
                    in_room: false,
                })),
                events: event_tx,
            },
        })
    }

    #[must_use]
    pub fn controller(&self) -> TrayController {
        self.controller.clone()
    }

    pub async fn recv_event(&mut self) -> Option<TrayEvent> {
        self.event_rx.recv().await
    }
}

impl Drop for SystemTray {
    fn drop(&mut self) {
        self.controller.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_event_wakes_the_receiver_and_the_status_is_kept() {
        let mut tray = SystemTray::spawn("Dekan Unit Test").expect("spawn tray");
        let controller = tray.controller();
        controller.update_status("Testing");
        assert_eq!(controller.status(), "Testing");
        controller.events().send(TrayEvent::Quit).expect("send");
        assert!(matches!(tray.recv_event().await, Some(TrayEvent::Quit)));
    }
}
