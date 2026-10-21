use super::*;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIN_BALLOONUSERCLICK, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, HICON, IDI_APPLICATION, LoadIconW, MF_DISABLED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    SetMenuDefaultItem, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WINDOW_EX_STYLE, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP,
    WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

struct TrayState {
    nid: NOTIFYICONDATAW,
    event_tx: UnboundedSender<TrayEvent>,
    status_text: String,
    status_shared: Arc<std::sync::Mutex<String>>,
    balloon_shared: Arc<std::sync::Mutex<Option<Balloon>>>,
}

pub(super) fn run_tray_message_loop(
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

    if unsafe { RegisterClassW(&wc) } == 0 {
        warn!(
            error = %windows::core::Error::from_thread(),
            "The tray window class could not be registered"
        );
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
        Err(e) => {
            error!(error = %e, "The tray window could not be created; Dekan will run without a tray icon");
            return;
        }
    };

    let _ = ready_tx.send(hwnd.0 as isize); // ignore-ok: nobody is waiting any more; the thread tears itself down below

    let hinstance = unsafe {
        windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default()
    };
    #[allow(clippy::manual_dangling_ptr)]
    let icon = unsafe {
        LoadIconW(Some(hinstance.into()), PCWSTR(1 as *const u16))
            .or_else(|e| {
                warn!(error = %e, "The Dekan tray icon resource could not be loaded; using the default icon");
                LoadIconW(None, IDI_APPLICATION)
            })
            .unwrap_or_else(|e| {
                warn!(error = %e, "No tray icon could be loaded; the tray entry will have a blank icon");
                HICON(std::ptr::null_mut())
            })
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
        loop {
            let got = GetMessageW(&mut msg, None, 0, 0);
            if got.0 == 0 {
                break;
            }
            if got.0 < 0 {
                error!(
                    error = %windows::core::Error::from_thread(),
                    "The tray message loop failed; the tray icon stops responding"
                );
                break;
            }
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
            state.status_text = state
                .status_shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
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
            LRESULT(0)
        }
        WM_TRAY_BALLOON => {
            let pending = state
                .balloon_shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
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

pub(super) fn fill_wide(target: &mut [u16], text: &str) {
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
            Err(e) => {
                warn!(error = %e, "The tray menu could not be created");
                return;
            }
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

        if !SetForegroundWindow(hwnd).as_bool() {
            debug!(
                "Windows refused the foreground for the tray menu; it may close on the first click"
            );
        }

        // ignore-ok: returns whether an item was chosen; the WM_COMMAND path handles the choice
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu); // ignore-ok: menu is discarded right after being shown
    }
}
