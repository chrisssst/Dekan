use tracing::warn;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};

use std::thread;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIN_BALLOONUSERCLICK, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, HICON, IDI_APPLICATION, LoadIconW, MF_DISABLED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetMenuDefaultItem, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WINDOW_EX_STYLE, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
    WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

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

    About,

    ToggleAutostart,

    ToggleAutoAccept,

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
            if let Ok(mut current) = self.status.lock() {
                *current = status.to_string();
            }
            let hwnd = HWND(self.hwnd_raw as *mut _);

            unsafe {
                let _ = PostMessageW(hwnd, WM_UPDATE_STATUS, WPARAM(0), LPARAM(0)); // ignore-ok: the target window is our own and `alive` was checked; a failure means it is already closed
            }
        }
    }

    #[must_use]
    pub fn status(&self) -> String {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    #[must_use]
    pub fn party(&self) -> (String, bool) {
        self.party
            .lock()
            .map(|p| (p.line.clone(), p.in_room))
            .unwrap_or_default()
    }

    #[must_use]
    pub fn events(&self) -> UnboundedSender<TrayEvent> {
        self.events.clone()
    }

    pub fn update_party(&self, line: &str, in_room: bool) {
        if let Ok(mut party) = self.party.lock() {
            party.line = line.to_string();
            party.in_room = in_room;
        }
    }

    pub fn notify(&self, title: &str, body: &str) {
        if !self.alive.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut pending) = self.balloon.lock() {
            *pending = Some(Balloon {
                title: title.to_string(),
                body: body.to_string(),
            });
        }
        let hwnd = HWND(self.hwnd_raw as *mut _);
        unsafe {
            let _ = PostMessageW(hwnd, WM_TRAY_BALLOON, WPARAM(0), LPARAM(0)); // ignore-ok: the target window is our own and `alive` was checked; a failure means it is already closed
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

struct TrayState {
    nid: NOTIFYICONDATAW,
    event_tx: UnboundedSender<TrayEvent>,
    status_text: String,
    status_shared: Arc<std::sync::Mutex<String>>,
    balloon_shared: Arc<std::sync::Mutex<Option<Balloon>>>,
}

fn run_tray_message_loop(
    title: String,
    event_tx: UnboundedSender<TrayEvent>,
    ready_tx: Sender<isize>,
    alive: Arc<AtomicBool>,
    status_shared: Arc<std::sync::Mutex<String>>,
    balloon_shared: Arc<std::sync::Mutex<Option<Balloon>>>,
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
    fill_wide(&mut nid.szTip, &initial_tip);

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
        balloon_shared,
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
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    let _ = state.event_tx.send(TrayEvent::Activated); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                NIN_BALLOONUSERCLICK => {
                    let _ = state.event_tx.send(TrayEvent::OpenRelease); // ignore-ok: the receiver is gone only when the app is already shutting down
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd_id = wparam.0 & 0xFFFF;
            match cmd_id {
                ID_OPEN_PANEL => {
                    let _ = state.event_tx.send(TrayEvent::Activated); // ignore-ok: the receiver is gone only when the app is already shutting down
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
                fill_wide(&mut state.nid.szTip, &tooltip);
                state.nid.uFlags = NIF_TIP;
                unsafe {
                    let _ = Shell_NotifyIconW(NIM_MODIFY, &state.nid); // ignore-ok: a refused status update leaves the previous tooltip text in place
                }
                state.nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            }
            LRESULT(0)
        }
        WM_TRAY_BALLOON => {
            let pending = state
                .balloon_shared
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            if let Some(balloon) = pending {
                fill_wide(&mut state.nid.szInfoTitle, &balloon.title);
                fill_wide(&mut state.nid.szInfo, &balloon.body);
                state.nid.dwInfoFlags = NIIF_INFO;
                state.nid.uFlags = NIF_INFO;
                unsafe {
                    if !Shell_NotifyIconW(NIM_MODIFY, &state.nid).as_bool() {
                        warn!(
                            "Windows refused the tray notification; the notice stays in the control panel"
                        );
                    }
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

fn fill_wide(target: &mut [u16], text: &str) {
    target.fill(0);
    let limit = target.len().saturating_sub(1);
    for (slot, unit) in target.iter_mut().zip(text.encode_utf16().take(limit)) {
        *slot = unit;
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
        let text = crate::i18n::text();
        let open_panel = HSTRING::from(text.menu_open_panel);
        let quit = HSTRING::from(text.menu_quit);

        // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(
            menu,
            MF_STRING | MF_GRAYED | MF_DISABLED,
            ID_STATUS_ITEM,
            PCWSTR(status_wide.as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN_PANEL, &open_panel); // ignore-ok: a menu item that fails to append is missing from the menu, which the user sees directly
        let _ = SetMenuDefaultItem(menu, ID_OPEN_PANEL as u32, 0); // ignore-ok: without a default item the entry is only shown in regular weight
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
