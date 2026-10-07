use std::sync::mpsc::channel;

use tokio::sync::mpsc::UnboundedSender;

use tracing::{debug, info, warn};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_B,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY, WM_USER,
};

use crate::tray::TrayEvent;

const MARK_PROBLEM_ID: i32 = 0xB11E;
const WM_ARM: u32 = WM_APP + 21;
const WM_DISARM: u32 = WM_APP + 22;

pub const MARK_PROBLEM_KEYS: &str = "Ctrl+Shift+B";

#[derive(Debug, Clone, Copy)]
pub struct MarkProblemHotkey {
    thread_id: u32,
}

impl MarkProblemHotkey {
    pub fn arm(&self) {
        self.post(WM_ARM);
    }

    pub fn disarm(&self) {
        self.post(WM_DISARM);
    }

    fn post(&self, message: u32) {
        if let Err(e) = unsafe { PostThreadMessageW(self.thread_id, message, WPARAM(0), LPARAM(0)) }
        {
            debug!(error = %e, "The mark-a-problem shortcut thread did not take the request");
        }
    }
}

fn register() -> bool {
    let registered = unsafe {
        RegisterHotKey(
            None,
            MARK_PROBLEM_ID,
            MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
            u32::from(VK_B.0),
        )
    };
    match registered {
        Ok(()) => {
            info!(
                keys = MARK_PROBLEM_KEYS,
                "Match shortcut active: marks a problem until the match ends"
            );
            true
        }
        Err(e) => {
            warn!(
                keys = MARK_PROBLEM_KEYS,
                error = %e,
                "The mark-a-problem shortcut is taken by another program; use the control panel button"
            );
            false
        }
    }
}

fn unregister() {
    if let Err(e) = unsafe { UnregisterHotKey(None, MARK_PROBLEM_ID) } {
        debug!(error = %e, "The mark-a-problem shortcut was already released");
    }
}

#[must_use]
pub fn spawn_mark_problem_hotkey(events: UnboundedSender<TrayEvent>) -> Option<MarkProblemHotkey> {
    let (ready_tx, ready_rx) = channel();
    let spawned = std::thread::Builder::new()
        .name("dekan-hotkey".into())
        .spawn(move || {
            let mut msg = MSG::default();
            unsafe {
                let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE); // ignore-ok: only creates this thread's message queue before its id is shared
            }
            let _ = ready_tx.send(unsafe { GetCurrentThreadId() }); // ignore-ok: the spawner gave up waiting; the thread idles until the process ends
            let mut armed = false;
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                match msg.message {
                    WM_ARM if !armed => armed = register(),
                    WM_DISARM if armed => {
                        unregister();
                        armed = false;
                        info!(keys = MARK_PROBLEM_KEYS, "Match shortcut released");
                    }
                    WM_HOTKEY
                        if msg.wParam.0 == MARK_PROBLEM_ID as usize
                            && events.send(TrayEvent::MarkProblem).is_err() =>
                    {
                        break;
                    }
                    _ => {}
                }
            }
            if armed {
                unregister();
            }
        });
    if let Err(e) = spawned {
        warn!(error = %e, "The mark-a-problem shortcut thread could not start; use the control panel button");
        return None;
    }
    match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(thread_id) => Some(MarkProblemHotkey { thread_id }),
        Err(e) => {
            warn!(error = %e, "The mark-a-problem shortcut thread did not start in time");
            None
        }
    }
}
