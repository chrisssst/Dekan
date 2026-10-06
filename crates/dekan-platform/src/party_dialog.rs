use std::num::NonZeroIsize;
use std::sync::{Arc, Mutex};

use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use tracing::{debug, warn};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH,
    PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, ICON_BIG, ICON_SMALL, IDC_ARROW, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, SendMessageW, ShowWindow, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_PAINT, WM_SETICON, WNDCLASSW, WS_CAPTION,
    WS_OVERLAPPED, WS_SYSMENU, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};
use wry::WebViewBuilder;

use crate::error::PlatformError;
use crate::welcome::load_dekan_icon;

const DIALOG_HTML: &str = include_str!("party_dialog_ui.html");

const WM_DIALOG_PASTE: u32 = WM_APP + 1;

struct DialogWindowHandle(HWND);

impl HasWindowHandle for DialogWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let non_zero = NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?;
        let handle = Win32WindowHandle::new(non_zero);
        let raw = RawWindowHandle::Win32(handle);
        unsafe { Ok(WindowHandle::borrow_raw(raw)) }
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn show_party_created_dialog(code: &str) -> Result<(), PlatformError> {
    let text = crate::i18n::text();
    run_dialog_modal(text.party_dialog_create_title, created_html(text, code)).map(|_| ())
}

pub(crate) fn created_html(text: &crate::i18n::Text, code: &str) -> String {
    let inline_btn = format!(
        "<button type=\"button\" class=\"btn-inline\" onclick=\"doCopy()\">{}</button>",
        escape_html(text.party_dialog_btn_copy)
    );
    let footer_btns = format!(
        "<button type=\"button\" class=\"btn btn-primary\" onclick=\"doSubmit()\">{}</button>",
        escape_html(text.party_dialog_btn_ok)
    );

    DIALOG_HTML
        .replace("{{lang}}", text.html_lang)
        .replace("{{title}}", &escape_html(text.party_dialog_create_title))
        .replace("{{desc}}", &escape_html(text.party_dialog_create_desc))
        .replace("{{label}}", &escape_html(text.party_dialog_label_code))
        .replace("{{initial_code}}", &escape_html(code))
        .replace("{{placeholder}}", "")
        .replace("{{readonly_attr}}", "readonly")
        .replace("{{action_inline_button}}", &inline_btn)
        .replace("{{footer_buttons}}", &footer_btns)
        .replace("{{copied_text}}", &escape_html(text.party_dialog_copied))
        .replace(
            "{{error_empty}}",
            &escape_html(text.party_dialog_error_empty),
        )
}

pub fn show_party_join_dialog(initial_code: Option<&str>) -> Result<Option<String>, PlatformError> {
    let text = crate::i18n::text();
    run_dialog_modal(text.party_dialog_join_title, join_html(text, initial_code))
}

pub(crate) fn join_html(text: &crate::i18n::Text, initial_code: Option<&str>) -> String {
    let inline_btn = format!(
        "<button type=\"button\" class=\"btn-inline\" onclick=\"doPaste()\">{}</button>",
        escape_html(text.party_dialog_btn_paste)
    );
    let footer_btns = format!(
        "<button type=\"button\" class=\"btn btn-secondary\" onclick=\"doCancel()\">{}</button>\
         <button type=\"button\" class=\"btn btn-primary\" onclick=\"doSubmit()\">{}</button>",
        escape_html(text.party_dialog_btn_cancel),
        escape_html(text.party_dialog_btn_join)
    );

    DIALOG_HTML
        .replace("{{lang}}", text.html_lang)
        .replace("{{title}}", &escape_html(text.party_dialog_join_title))
        .replace("{{desc}}", &escape_html(text.party_dialog_join_desc))
        .replace("{{label}}", &escape_html(text.party_dialog_label_code))
        .replace("{{initial_code}}", &escape_html(initial_code.unwrap_or("")))
        .replace(
            "{{placeholder}}",
            &escape_html(text.party_dialog_placeholder),
        )
        .replace("{{readonly_attr}}", "")
        .replace("{{action_inline_button}}", &inline_btn)
        .replace("{{footer_buttons}}", &footer_btns)
        .replace("{{copied_text}}", &escape_html(text.party_dialog_copied))
        .replace(
            "{{error_empty}}",
            &escape_html(text.party_dialog_error_empty),
        )
}

fn run_dialog_modal(title: &str, html: String) -> Result<Option<String>, PlatformError> {
    let class_name = w!("DekanPartyDialogClass");
    let hicon = load_dekan_icon().unwrap_or_default();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(dialog_wnd_proc),
        hInstance: Default::default(),
        lpszClassName: class_name,

        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hIcon: hicon,
        hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
        ..Default::default()
    };

    unsafe {
        let _ = RegisterClassW(&wc); // ignore-ok: failure just means already registered
    }

    let width = 520;
    let height = 280;

    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };

    let pos_x = (screen_w - width) / 2;
    let pos_y = (screen_h - height) / 2;

    let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();

    let hwnd = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR(title_wide.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            pos_x,
            pos_y,
            width,
            height,
            None,
            None,
            None,
            None,
        )
    } {
        Ok(h) => h,
        Err(e) => {
            warn!(error = %e, "Party dialog: could not create the window");
            return Err(PlatformError::Window(format!(
                "failed to create dialog window: {e}"
            )));
        }
    };

    if !hicon.is_invalid() {
        unsafe {
            // ignore-ok: WM_SETICON returns previous icon
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_BIG as usize),
                LPARAM(hicon.0 as isize),
            );

            // ignore-ok: WM_SETICON returns previous icon
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_SMALL as usize),
                LPARAM(hicon.0 as isize),
            );
        }
    }

    let dark_mode_val: i32 = 1;

    unsafe {
        // ignore-ok: cosmetic titlebar dark mode
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode_val as *const _ as *const _,
            std::mem::size_of::<i32>() as u32,
        );
    }

    crate::paths::ensure_webview2_data_dir();

    let mut client_rect = RECT::default();

    unsafe {
        // ignore-ok: zeroed rect falls back
        let _ = GetClientRect(hwnd, &mut client_rect);
    }
    let client_w = (client_rect.right - client_rect.left) as u32;
    let client_h = (client_rect.bottom - client_rect.top) as u32;

    let host = DialogWindowHandle(hwnd);
    let hwnd_raw = hwnd.0 as isize;

    let result_storage = Arc::new(Mutex::new(None));
    let result_for_ipc = Arc::clone(&result_storage);

    let pending_paste: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let paste_for_ipc = Arc::clone(&pending_paste);

    let webview = WebViewBuilder::new()
        .with_html(html)
        .with_bounds(wry::Rect {
            position: wry::dpi::LogicalPosition::new(0, 0).into(),
            size: wry::dpi::LogicalSize::new(client_w, client_h).into(),
        })
        .with_ipc_handler(move |request| {
            let body = request.body();
            if body == "cancel" {
                if let Ok(mut res) = result_for_ipc.lock() {
                    *res = None;
                }

                unsafe {

                    // ignore-ok: window closing
                    let _ = PostMessageW(HWND(hwnd_raw as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            } else if let Some(code) = body.strip_prefix("submit:") {
                let code = code.trim().to_string();
                if let Ok(mut res) = result_for_ipc.lock() {
                    *res = Some(code);
                }

                unsafe {

                    // ignore-ok: window closing
                    let _ = PostMessageW(HWND(hwnd_raw as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            } else if let Some(code) = body.strip_prefix("copy:") {

                if let Err(e) = crate::clipboard::set_text(code) {
                    warn!(error = %e, "Party dialog: could not copy the room code to the clipboard");
                }
            } else if body == "request_paste" {

                paste_from_clipboard(&paste_for_ipc, HWND(hwnd_raw as *mut _));
            }
        })
        .build_as_child(&host);

    let webview = match webview {
        Ok(webview) => webview,
        Err(e) => {
            warn!(error = %e, "Party dialog: could not create the WebView");

            unsafe {
                // ignore-ok: teardown on error
                let _ = DestroyWindow(hwnd);
            }
            return Err(PlatformError::Window(format!(
                "failed to initialize webview: {e}"
            )));
        }
    };

    unsafe {
        // ignore-ok: ShowWindow returns previous visibility
        let _ = ShowWindow(hwnd, SW_SHOW);
    }

    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();

    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_DIALOG_PASTE {
                apply_pending_paste(&webview, &pending_paste);
                continue;
            }

            // ignore-ok: returns whether key was translated
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let final_result = result_storage.lock().map(|res| res.clone()).unwrap_or(None);
    Ok(final_result)
}

fn paste_from_clipboard(pending: &Mutex<Option<String>>, hwnd: HWND) {
    let text = match crate::clipboard::get_text() {
        Ok(Some(text)) => text,
        Ok(None) => {
            debug!("Party dialog: paste requested but the clipboard holds no text");
            return;
        }
        Err(e) => {
            warn!(error = %e, "Party dialog: could not read the clipboard for paste");
            return;
        }
    };
    match pending.lock() {
        Ok(mut slot) => *slot = Some(text),
        Err(e) => {
            warn!(error = %e, "Party dialog: paste hand-off is poisoned; paste dropped");
            return;
        }
    }

    if let Err(e) = unsafe { PostMessageW(hwnd, WM_DIALOG_PASTE, WPARAM(0), LPARAM(0)) } {
        warn!(error = %e, "Party dialog: could not hand the pasted text to the page");
    }
}

fn apply_pending_paste(webview: &wry::WebView, pending: &Mutex<Option<String>>) {
    let text = match pending.lock() {
        Ok(mut slot) => slot.take(),
        Err(e) => {
            warn!(error = %e, "Party dialog: paste hand-off is poisoned; paste dropped");
            return;
        }
    };
    let Some(text) = text else { return };

    let literal = match serde_json::to_string(&text) {
        Ok(literal) => literal,
        Err(e) => {
            warn!(error = %e, "Party dialog: could not encode the pasted text");
            return;
        }
    };
    if let Err(e) = webview.evaluate_script(&format!("applyPasteFromRust({literal})")) {
        warn!(error = %e, "Party dialog: could not deliver the pasted text to the page");
    }
}

unsafe extern "system" fn dialog_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLOSE => {
            unsafe {
                // ignore-ok: window destruction
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();

            unsafe {
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rect = RECT::default();

                // ignore-ok: zeroed rect
                let _ = GetClientRect(hwnd, &mut rect);
                let bg_brush = CreateSolidBrush(COLORREF(0x000C0805));
                FillRect(hdc, &rect, bg_brush);

                // ignore-ok: GDI cleanup
                let _ = DeleteObject(bg_brush);

                // ignore-ok: EndPaint always returns TRUE
                let _ = EndPaint(hwnd, &ps);
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
