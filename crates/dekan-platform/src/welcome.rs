use std::num::NonZeroIsize;
use std::thread;

use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use tracing::warn;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, HICON, ICON_BIG, ICON_SMALL, IDC_ARROW, LoadCursorW, LoadIconW, MINMAXINFO,
    PostMessageW, PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, SendMessageW,
    SetTimer, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_CREATE,
    WM_DESTROY, WM_GETMINMAXINFO, WM_PAINT, WM_SETICON, WM_SIZE, WM_TIMER, WNDCLASSW, WS_CAPTION,
    WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};
use wry::WebViewBuilder;

const WELCOME_HTML: &str = include_str!("welcome_ui.html");
const TIMER_AUTO_DISMISS: usize = 2001;

/// Posted by the window procedure on `WM_SIZE`: the WebView lives on the pump, not in the proc.
const WM_WELCOME_RESIZED: u32 = WM_APP + 1;

const WELCOME_WIDTH: i32 = 530;
const WELCOME_HEIGHT: i32 = 440;
const WELCOME_MIN_WIDTH: i32 = 420;
const WELCOME_MIN_HEIGHT: i32 = 340;

/// One list row, or nothing: an empty string must not leave a bare dekan behind.
fn list_item(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format!(
            "<li><span class=\"dekan\">&#9670;</span><span>{}</span></li>",
            escape_html(value)
        )
    }
}

fn welcome_html(text: &crate::i18n::Text) -> String {
    WELCOME_HTML
        .replace("{{lang}}", text.html_lang)
        .replace("{{version}}", crate::version::display_version())
        .replace("{{welcome_active}}", &escape_html(text.welcome_active))
        .replace(
            "{{welcome_background}}",
            &escape_html(text.welcome_background),
        )
        .replace("{{welcome_author}}", &escape_html(text.welcome_author))
        .replace(
            "{{welcome_tray_hint_item}}",
            &list_item(text.welcome_tray_hint),
        )
        .replace("{{welcome_dismiss}}", &escape_html(text.welcome_dismiss))
        .replace("{{welcome_quote_item}}", &list_item(text.welcome_quote))
}

fn about_html(text: &crate::i18n::Text) -> String {
    WELCOME_HTML
        .replace("{{lang}}", text.html_lang)
        .replace("{{version}}", crate::version::display_version())
        .replace("{{welcome_active}}", &escape_html(text.about_title))
        .replace(
            "{{welcome_background}}",
            &escape_html(text.about_educational),
        )
        .replace("{{welcome_author}}", &escape_html(text.welcome_author))
        .replace("{{welcome_tray_hint_item}}", "")
        .replace("{{welcome_dismiss}}", &escape_html(text.about_dismiss))
        .replace("{{welcome_quote_item}}", &list_item(text.about_quote))
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

struct WelcomeWindowHandle(HWND);

impl HasWindowHandle for WelcomeWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let non_zero = NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?;
        let handle = Win32WindowHandle::new(non_zero);
        let raw = RawWindowHandle::Win32(handle);
        // SAFETY: self.0 is a valid Win32 HWND owned by this thread for the lifetime of the borrow.
        unsafe { Ok(WindowHandle::borrow_raw(raw)) }
    }
}

/// Load the embedded Dekan application icon from the PE resources.
#[allow(clippy::manual_dangling_ptr)]
pub(crate) fn load_dekan_icon() -> Option<HICON> {
    unsafe {
        if let Ok(module) = GetModuleHandleW(None) {
            // Resource ID 1 is standard for the icon embedded by winres in build.rs
            if let Ok(icon) = LoadIconW(HINSTANCE(module.0), PCWSTR(1 as *const u16)) {
                if !icon.is_invalid() {
                    return Some(icon);
                }
            }
        }
    }
    None
}

/// Spawn the styled dark-mode welcome window on a background thread (auto-dismisses in 15s).
pub fn show_welcome_window() {
    thread::spawn(|| {
        run_welcome_window_pump(true, false);
    });
}

/// Spawn the styled dark-mode About window on a background thread (stays open until user dismisses).
pub fn show_about_window() {
    thread::spawn(|| {
        run_welcome_window_pump(false, true);
    });
}

fn run_welcome_window_pump(auto_dismiss: bool, is_about: bool) {
    let class_name = w!("DekanWelcomeWindowClass");
    let hicon = load_dekan_icon().unwrap_or_default();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(welcome_wnd_proc),
        hInstance: Default::default(),
        lpszClassName: class_name,
        // SAFETY: Loading standard arrow cursor
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hIcon: hicon,
        hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
        ..Default::default()
    };

    // SAFETY: RegisterClassW registers our welcome window class
    unsafe {
        let _ = RegisterClassW(&wc); // ignore-ok: failure just means already registered
    }

    let width = WELCOME_WIDTH;
    let height = WELCOME_HEIGHT;

    // SAFETY: GetSystemMetrics queries screen resolution
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };

    let pos_x = (screen_w - width) / 2;
    let pos_y = (screen_h - height) / 2;

    // SAFETY: CreateWindowExW creates the top-level welcome frame
    let hwnd = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("Dekan — League of Legends Skin Changer"),
            WS_OVERLAPPED
                | WS_CAPTION
                | WS_SYSMENU
                | WS_THICKFRAME
                | WS_MINIMIZEBOX
                | WS_MAXIMIZEBOX
                | WS_VISIBLE,
            pos_x,
            pos_y,
            width,
            height,
            None,
            None,
            None,
            Some(auto_dismiss as usize as *const _),
        )
    } {
        Ok(h) => h,
        Err(e) => {
            warn!(error = %e, "Welcome window: could not create the window");
            return;
        }
    };

    // Ensure window icons are explicitly assigned to titlebar and taskbar
    if !hicon.is_invalid() {
        // SAFETY: SendMessageW with WM_SETICON assigns icon to the window
        unsafe {
            // ignore-ok: WM_SETICON returns the previous icon, not a status; a default icon is cosmetic
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_BIG as usize),
                LPARAM(hicon.0 as isize),
            );
            // ignore-ok: same call for the small icon; see above.
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_SMALL as usize),
                LPARAM(hicon.0 as isize),
            );
        }
    }

    // Enable Windows 10/11 Immersive Dark Mode for titlebar
    let dark_mode_val: i32 = 1;
    // SAFETY: DwmSetWindowAttribute is cosmetic; older Windows builds simply refuse
    unsafe {
        // ignore-ok: cosmetic; older Windows builds refuse this attribute by design
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode_val as *const _ as *const _,
            std::mem::size_of::<i32>() as u32,
        );
    }

    crate::paths::ensure_webview2_data_dir();

    let mut client_rect = RECT::default();
    // SAFETY: GetClientRect queries client area of hwnd
    unsafe {
        // ignore-ok: a zeroed rect creates the WebView at 0x0, resized by the first WM_SIZE
        let _ = GetClientRect(hwnd, &mut client_rect);
    }
    let client_w = (client_rect.right - client_rect.left) as u32;
    let client_h = (client_rect.bottom - client_rect.top) as u32;

    let host = WelcomeWindowHandle(hwnd);
    let hwnd_raw = hwnd.0 as isize;
    let html = if is_about {
        about_html(crate::i18n::text())
    } else {
        welcome_html(crate::i18n::text())
    };
    let webview = WebViewBuilder::new()
        .with_html(html)
        .with_bounds(wry::Rect {
            position: wry::dpi::LogicalPosition::new(0, 0).into(),
            size: wry::dpi::LogicalSize::new(client_w, client_h).into(),
        })
        .with_ipc_handler(move |request| {
            if request.body() == "dismiss" {
                // SAFETY: PostMessageW safely requests window close on dismiss button click
                unsafe {
                    // ignore-ok: best-effort dismiss; the 15 s timer and titlebar still close it
                    let _ = PostMessageW(HWND(hwnd_raw as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        })
        .build_as_child(&host);
    // Kept alive until the pump ends: dropping it tears the page down.
    let webview = match webview {
        Ok(webview) => webview,
        Err(e) => {
            // An empty frame is worse than no window: the welcome is informational only.
            warn!(error = %e, "Welcome window: could not create the WebView");
            // SAFETY: destroying this thread's own window on a failed build.
            unsafe {
                // ignore-ok: the only failure is an already-gone window, which is the wanted outcome
                let _ = DestroyWindow(hwnd);
            }
            return;
        }
    };

    unsafe {
        // ignore-ok: ShowWindow returns the previous visibility, not a status.
        let _ = ShowWindow(hwnd, SW_SHOW);
    }

    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();

    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_WELCOME_RESIZED {
                let mut rect = RECT::default();
                if GetClientRect(hwnd, &mut rect).is_ok() {
                    let bounds = wry::Rect {
                        position: wry::dpi::LogicalPosition::new(0, 0).into(),
                        size: wry::dpi::LogicalSize::new(
                            (rect.right - rect.left).max(0) as u32,
                            (rect.bottom - rect.top).max(0) as u32,
                        )
                        .into(),
                    };
                    if let Err(e) = webview.set_bounds(bounds) {
                        tracing::debug!(error = %e, "Welcome window: could not resize the WebView");
                    }
                }
                continue;
            }
            // ignore-ok: reports whether a key message was translated; this window takes none
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn welcome_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let createstruct =
                lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            let auto_dismiss = if !createstruct.is_null() {
                unsafe { (*createstruct).lpCreateParams as usize != 0 }
            } else {
                false
            };
            if auto_dismiss {
                unsafe {
                    // ignore-ok: without the timer the window waits for the user instead of self-dismissing
                    let _ = SetTimer(hwnd, TIMER_AUTO_DISMISS, 15_000, None);
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                // ignore-ok: the only failure is an already-gone window, which is the wanted outcome
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            unsafe {
                // ignore-ok: our own window; a lost resize is redone by the next WM_SIZE
                let _ = PostMessageW(hwnd, WM_WELCOME_RESIZED, WPARAM(0), LPARAM(0));
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = lparam.0 as *mut MINMAXINFO;
            if !info.is_null() {
                // SAFETY: Windows passes a valid MINMAXINFO for this message.
                unsafe {
                    (*info).ptMinTrackSize.x = WELCOME_MIN_WIDTH;
                    (*info).ptMinTrackSize.y = WELCOME_MIN_HEIGHT;
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_AUTO_DISMISS {
                unsafe {
                    // ignore-ok: see WM_CLOSE — already destroyed is the wanted state.
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();

            unsafe {
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rect = RECT::default();

                // ignore-ok: a zeroed rect paints nothing; this fill is only the WebView backdrop
                let _ = GetClientRect(hwnd, &mut rect);

                let bg_brush = CreateSolidBrush(COLORREF(0x000C0805));
                FillRect(hdc, &rect, bg_brush);

                // ignore-ok: leaks one GDI brush for the life of a window that closes in 15 s
                let _ = DeleteObject(bg_brush);

                // ignore-ok: EndPaint always returns TRUE for a valid PAINTSTRUCT from BeginPaint.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn about_page_is_distinct_from_the_intro_and_states_it_is_educational() {
        for language in [Language::Portuguese, Language::Spanish, Language::English] {
            let text = language.text();
            let about = about_html(text);
            let welcome = welcome_html(text);
            assert!(
                !about.contains("{{"),
                "{language:?} about left a placeholder"
            );
            assert_ne!(
                about, welcome,
                "{language:?}: About must differ from the intro"
            );
            assert!(
                about.contains(&escape_html(text.about_title)),
                "{language:?}: About shows its own title"
            );
            assert!(
                about.contains("Miss Fortune"),
                "{language:?}: About carries the Miss Fortune line"
            );

            assert!(!about.contains(&escape_html(text.welcome_tray_hint)));
            assert!(
                !about.contains("<span></span>"),
                "{language:?}: About must not render an empty list row"
            );
        }
    }

    #[test]
    fn every_language_fills_every_placeholder() {
        for language in [Language::Portuguese, Language::Spanish, Language::English] {
            let html = welcome_html(language.text());
            assert!(!html.contains("{{"), "{language:?} left a placeholder");
            assert!(html.contains(language.text().welcome_dismiss));
        }
        assert!(
            !welcome_html(Language::English.text()).contains("Miss Fortune"),
            "no guessed translation of the quote"
        );
    }
}
