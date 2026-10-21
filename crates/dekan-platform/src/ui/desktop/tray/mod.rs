use tracing::{debug, error, warn};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};

use std::thread;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_USER};

use crate::error::PlatformError;

const WM_TRAY_CALLBACK: u32 = WM_USER + 100;
const WM_UPDATE_STATUS: u32 = WM_USER + 101;
const WM_TRAY_QUIT: u32 = WM_USER + 102;
const WM_TRAY_BALLOON: u32 = WM_USER + 103;

const ID_STATUS_ITEM: usize = 1001;
const ID_QUIT: usize = 1003;
const ID_OPEN_PANEL: usize = 1013;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    OpenLogs,

    OpenTools,

    OpenMods,

    RestoreMods,

    About,

    ToggleAutostart,

    ToggleAutoAccept,

    ToggleInjection,

    ToggleRandomSkin,
    ToggleLightLoading,

    Quit,

    Activated,

    PartyCreate,

    PartyJoin,

    PartyLeave,

    OpenRelease,
    InstallInjector,

    MarkProblem,

    ExportDiagnostics,
}

#[derive(Clone)]
pub struct TrayController {
    hwnd_raw: isize,
    alive: Arc<AtomicBool>,
    status: Arc<std::sync::Mutex<String>>,
    party: Arc<std::sync::Mutex<PartyMenu>>,
    balloon: Arc<std::sync::Mutex<Option<Balloon>>>,
    events: UnboundedSender<TrayEvent>,
}

#[derive(Debug, Clone)]
struct Balloon {
    title: String,
    body: String,
}

#[derive(Debug, Clone, Default)]
struct PartyMenu {
    line: String,
    in_room: bool,
}

impl TrayController {
    pub fn update_status(&self, status: &str) {
        if self.alive.load(Ordering::Relaxed) {
            *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status.to_string();
            let hwnd = HWND(self.hwnd_raw as *mut _);

            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_UPDATE_STATUS, WPARAM(0), LPARAM(0)); // ignore-ok: the target window is our own and `alive` was checked; a failure means it is already closed
            }
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
        *self.balloon.lock().unwrap_or_else(|e| e.into_inner()) = Some(Balloon {
            title: title.to_string(),
            body: body.to_string(),
        });
        let hwnd = HWND(self.hwnd_raw as *mut _);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_TRAY_BALLOON, WPARAM(0), LPARAM(0)); // ignore-ok: the target window is our own and `alive` was checked; a failure means it is already closed
        }
    }

    pub fn shutdown(&self) {
        if self.alive.swap(false, Ordering::SeqCst) {
            let hwnd = HWND(self.hwnd_raw as *mut _);
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_TRAY_QUIT, WPARAM(0), LPARAM(0)); // ignore-ok: the target window is our own and `alive` was checked; a failure means it is already closed
            }
        }
    }
}

pub struct SystemTray {
    event_rx: UnboundedReceiver<TrayEvent>,
    controller: TrayController,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl SystemTray {
    pub fn spawn(initial_title: &str) -> Result<Self, PlatformError> {
        let (event_tx, event_rx) = unbounded_channel();
        let (ready_tx, ready_rx) = channel();

        let title_owned = initial_title.to_string();
        let alive_flag = Arc::new(AtomicBool::new(true));
        let alive_for_thread = Arc::clone(&alive_flag);

        let status_shared = Arc::new(std::sync::Mutex::new(initial_title.to_string()));
        let status_for_thread = Arc::clone(&status_shared);
        let party_shared = Arc::new(std::sync::Mutex::new(PartyMenu {
            line: crate::i18n::text().party_off.into(),
            in_room: false,
        }));
        let events_for_controller = event_tx.clone();
        let balloon_shared = Arc::new(std::sync::Mutex::new(None));
        let balloon_for_thread = Arc::clone(&balloon_shared);

        let join_handle = thread::Builder::new()
            .name("dekan-tray-pump".into())
            .spawn(move || {
                run_tray_message_loop(
                    title_owned,
                    event_tx,
                    ready_tx,
                    alive_for_thread,
                    status_for_thread,
                    balloon_for_thread,
                );
            })
            .map_err(|e| PlatformError::Io {
                context: "failed to spawn system tray thread".into(),
                source: e,
            })?;

        let hwnd_raw = ready_rx.recv().map_err(|_| PlatformError::Io {
            context: "tray thread initialization failed".into(),
            source: std::io::Error::other("channel closed"),
        })?;

        Ok(Self {
            event_rx,
            controller: TrayController {
                hwnd_raw,
                alive: alive_flag,
                status: status_shared,
                party: party_shared,
                balloon: balloon_shared,
                events: events_for_controller,
            },
            join_handle: Some(join_handle),
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
        if let Some(handle) = self.join_handle.take() {
            if let Err(e) = handle.join() {
                warn!(panic = ?e, "Tray message thread panicked");
            }
        }
    }
}

mod window;

use window::run_tray_message_loop;

#[cfg(test)]
mod tests;
