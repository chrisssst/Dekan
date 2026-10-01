use tracing::warn;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, HICON, IDI_APPLICATION, LoadIconW, MF_CHECKED,
    MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WINDOW_EX_STYLE, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP,
    WM_USER, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::error::PlatformError;

const WM_TRAY_CALLBACK: u32 = WM_USER + 100;
const WM_UPDATE_STATUS: u32 = WM_USER + 101;
const WM_TRAY_QUIT: u32 = WM_USER + 102;

const ID_STATUS_ITEM: usize = 1001;
const ID_OPEN_LOGS: usize = 1002;
const ID_QUIT: usize = 1003;
const ID_PARTY_STATUS: usize = 1004;
const ID_PARTY_CREATE: usize = 1005;
const ID_PARTY_JOIN: usize = 1006;
const ID_PARTY_LEAVE: usize = 1007;
const ID_OPEN_TOOLS: usize = 1008;
const ID_OPEN_MODS: usize = 1009;
const ID_ABOUT: usize = 1010;
const ID_AUTOSTART: usize = 1011;
const ID_AUTO_ACCEPT: usize = 1012;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    OpenLogs,

    OpenTools,

    OpenMods,

    About,

    ToggleAutostart,

    ToggleAutoAccept,

    Quit,

    Activated,

    PartyCreate,

    PartyJoin,

    PartyLeave,
}

#[derive(Clone)]
pub struct TrayController {
    hwnd_raw: isize,
    alive: Arc<AtomicBool>,
    status: Arc<std::sync::Mutex<String>>,
    party: Arc<std::sync::Mutex<PartyMenu>>,
}

#[derive(Debug, Clone, Default)]
struct PartyMenu {
    line: String,
    in_room: bool,
}

impl TrayController {
    pub fn update_status(&self, status: &str) {
        if self.alive.load(Ordering::Relaxed) {
            if let Ok(mut current) = self.status.lock() {
                *current = status.to_string();
            }
            let hwnd = HWND(self.hwnd_raw as *mut _);

            unsafe {
                let _ = PostMessageW(hwnd, WM_UPDATE_STATUS, WPARAM(0), LPARAM(0)); // ignore-ok: the target window is our own and `alive` was checked; a failure means it is already closed
            }
        }
    }

    pub fn update_party(&self, line: &str, in_room: bool) {
        if let Ok(mut party) = self.party.lock() {
            party.line = line.to_string();
            party.in_room = in_room;
        }
    }

    pub fn shutdown(&self) {
        if self.alive.swap(false, Ordering::SeqCst) {
            let hwnd = HWND(self.hwnd_raw as *mut _);
            unsafe {
                let _ = PostMessageW(hwnd, WM_TRAY_QUIT, WPARAM(0), LPARAM(0)); // ignore-ok: the target window is our own and `alive` was checked; a failure means it is already closed
            }
        }
    }
}

pub struct SystemTray {
    event_rx: Receiver<TrayEvent>,
    controller: TrayController,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl SystemTray {
    pub fn spawn(initial_title: &str) -> Result<Self, PlatformError> {
        let (event_tx, event_rx) = channel();
        let (ready_tx, ready_rx) = channel();

        let title_owned = initial_title.to_string();
        let alive_flag = Arc::new(AtomicBool::new(true));
        let alive_for_thread = Arc::clone(&alive_flag);

        let status_shared = Arc::new(std::sync::Mutex::new(initial_title.to_string()));
        let status_for_thread = Arc::clone(&status_shared);
        let party_shared = Arc::new(std::sync::Mutex::new(PartyMenu {
            line: "Party: desligado".into(),
            in_room: false,
        }));
        let party_for_thread = Arc::clone(&party_shared);

        let join_handle = thread::Builder::new()
            .name("dekan-tray-pump".into())
            .spawn(move || {
                run_tray_message_loop(
                    title_owned,
                    event_tx,
                    ready_tx,
                    alive_for_thread,
                    status_for_thread,
                    party_for_thread,
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
            },
            join_handle: Some(join_handle),
        })
    }

    #[must_use]
    pub fn controller(&self) -> TrayController {
        self.controller.clone()
    }

    pub fn try_recv_event(&self) -> Option<TrayEvent> {
        self.event_rx.try_recv().ok()
    }

    pub fn recv_event(&self) -> Option<TrayEvent> {
        self.event_rx.recv().ok()
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

struct TrayState {
    nid: NOTIFYICONDATAW,
    event_tx: Sender<TrayEvent>,
    status_text: String,
    status_shared: Arc<std::sync::Mutex<String>>,
    party_shared: Arc<std::sync::Mutex<PartyMenu>>,
}

fn run_tray_message_loop(
    title: String,
    event_tx: Sender<TrayEvent>,
    ready_tx: Sender<isize>,
    alive: Arc<AtomicBool>,
    status_shared: Arc<std::sync::Mutex<String>>,
    party_shared: Arc<std::sync::Mutex<PartyMenu>>,
) {
    let class_name = w!("DekanTrayWindowClass");

    let wc = WNDCLASSW {
        lpfnWndProc: Some(tray_wnd_proc),
        hInstance: Default::default(),
        lpszClassName: class_name,
        ..Default::default()
    };

    unsafe {
        RegisterClassW(&wc);
    }

    let hwnd = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("Dekan Tray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )
    } {
        Ok(h) => h,
        Err(_) => return,
    };

    let _ = ready_tx.send(hwnd.0 as isize); // ignore-ok: nobody is waiting any more; the thread tears itself down below

    let hinstance = unsafe {
        windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default()
    };
    #[allow(clippy::manual_dangling_ptr)]
    let icon = unsafe {
        LoadIconW(hinstance, PCWSTR(1 as *const u16))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or(HICON(std::ptr::null_mut()))
    };

    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY_CALLBACK,
        hIcon: icon,
        ..Default::default()
    };

    let initial_tip = format!("Dekan v{}: {}", crate::version::display_version(), title);
    let tip_wide: Vec<u16> = initial_tip
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let copy_len = tip_wide.len().min(nid.szTip.len());
    nid.szTip[..copy_len].copy_from_slice(&tip_wide[..copy_len]);

    unsafe {
        if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
            warn!("Shell_NotifyIcon refused to add the tray icon; Dekan will run without one");
        }
    }

    let mut state = Box::new(TrayState {
        nid,
        event_tx,
        status_text: title,
        status_shared,
        party_shared,
    });

    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
            (&mut *state as *mut TrayState) as isize,
        );
    }

    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg); // ignore-ok: returns whether a key event was translated; this pump forwards either way
            DispatchMessageW(&msg);
        }
    }

    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &state.nid); // ignore-ok: removing an icon while shutting down; if it is already gone, so much the better
        let _ = DestroyWindow(hwnd); // ignore-ok: the window is being torn down; a failure means it is already gone
    }

    alive.store(false, Ordering::SeqCst);
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
        )
    } as *mut TrayState;

    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }

    let state = unsafe { &mut *ptr };

    match msg {
        WM_TRAY_CALLBACK => {
            let event = lparam.0 as u32;
            match event {
                WM_RBUTTONUP => {
                    show_context_menu(hwnd, state);
                }
                WM_LBUTTONDBLCLK => {
                    let _ = state.event_tx.send(TrayEvent::Activated); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd_id = wparam.0 & 0xFFFF;
            match cmd_id {
                ID_OPEN_MODS => {
                    let _ = state.event_tx.send(TrayEvent::OpenMods); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                ID_OPEN_LOGS => {
                    let _ = state.event_tx.send(TrayEvent::OpenLogs); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                ID_OPEN_TOOLS => {
                    let _ = state.event_tx.send(TrayEvent::OpenTools); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                ID_ABOUT => {
                    let _ = state.event_tx.send(TrayEvent::About); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                ID_AUTOSTART => {
                    let _ = state.event_tx.send(TrayEvent::ToggleAutostart); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                ID_AUTO_ACCEPT => {
                    let _ = state.event_tx.send(TrayEvent::ToggleAutoAccept); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                ID_PARTY_CREATE => {
                    let _ = state.event_tx.send(TrayEvent::PartyCreate); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                ID_PARTY_JOIN => {
                    let _ = state.event_tx.send(TrayEvent::PartyJoin); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                ID_PARTY_LEAVE => {
                    let _ = state.event_tx.send(TrayEvent::PartyLeave); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                ID_QUIT => {
                    let _ = state.event_tx.send(TrayEvent::Quit); // ignore-ok: the receiver is gone only when the app is already shutting down
                    unsafe {
                        PostQuitMessage(0);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_UPDATE_STATUS => {
            if let Ok(current) = state.status_shared.lock() {
                state.status_text = current.clone();
                let tooltip = format!(
                    "Dekan v{}: {}",
                    crate::version::display_version(),
                    state.status_text
                );
                let tip_wide: Vec<u16> = tooltip.encode_utf16().chain(std::iter::once(0)).collect();
                state.nid.szTip = [0; 128];
                let copy_len = tip_wide.len().min(state.nid.szTip.len());
                state.nid.szTip[..copy_len].copy_from_slice(&tip_wide[..copy_len]);
                state.nid.uFlags = NIF_TIP;
                unsafe {
                    let _ = Shell_NotifyIconW(NIM_MODIFY, &state.nid); // ignore-ok: a refused status update leaves the previous tooltip text in place
                }
                state.nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            }
            LRESULT(0)
        }
        WM_TRAY_QUIT => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn show_context_menu(hwnd: HWND, state: &TrayState) {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };

        let status_item = format!(
            "Dekan v{}: {}",
            crate::version::display_version(),
            state.status_text
        );
        let status_wide: Vec<u16> = status_item
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(
            menu,
            MF_STRING | MF_GRAYED | MF_DISABLED,
            ID_STATUS_ITEM,
            PCWSTR(status_wide.as_ptr()),
        );

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly

        let party = state
            .party_shared
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default();
        let party_wide: Vec<u16> = party
            .line
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(
            menu,
            MF_STRING | MF_GRAYED | MF_DISABLED,
            ID_PARTY_STATUS,
            PCWSTR(party_wide.as_ptr()),
        );

        let text = crate::i18n::text();
        let create = HSTRING::from(text.menu_party_create);
        let join = HSTRING::from(text.menu_party_join);
        let leave = HSTRING::from(text.menu_party_leave);
        let open_mods = HSTRING::from(text.menu_open_mods);
        let open_logs = HSTRING::from(text.menu_open_logs);
        let open_tools = HSTRING::from(text.menu_open_tools);
        let about = HSTRING::from(text.menu_about);
        let autostart = HSTRING::from(text.menu_autostart);
        let auto_accept = HSTRING::from(text.menu_auto_accept);
        let quit = HSTRING::from(text.menu_quit);

        let _ = AppendMenuW(menu, MF_STRING, ID_PARTY_CREATE, &create); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(menu, MF_STRING, ID_PARTY_JOIN, &join); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let leave_flags = if party.in_room {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED | MF_DISABLED
        };
        let _ = AppendMenuW(menu, leave_flags, ID_PARTY_LEAVE, &leave); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly

        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN_MODS, &open_mods); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN_LOGS, &open_logs); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN_TOOLS, &open_tools); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly

        let autostart_enabled = match crate::autostart::is_enabled() {
            Ok(enabled) => enabled,
            Err(e) => {
                warn!(error = %e, "Could not read the Start with Windows setting; shown as off");
                false
            }
        };
        let autostart_check = if autostart_enabled {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let auto_accept_check = if crate::auto_accept::is_enabled() {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(
            menu,
            MF_STRING | auto_accept_check,
            ID_AUTO_ACCEPT,
            &auto_accept,
        );
        let _ = AppendMenuW(menu, MF_STRING | autostart_check, ID_AUTOSTART, &autostart); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(menu, MF_STRING, ID_ABOUT, &about); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, &quit); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt); // ignore-ok: menu falls back to position 0,0 instead of not opening

        let _ = SetForegroundWindow(hwnd); // ignore-ok: focus is advisory; Windows refuses it by policy in several states

        // ignore-ok: returns whether an item was chosen; the WM_COMMAND path handles the choice
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            0,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu); // ignore-ok: menu is discarded right after being shown
    }
}

#[cfg(test)]
#[path = "tray_tests.rs"]
mod tests;
