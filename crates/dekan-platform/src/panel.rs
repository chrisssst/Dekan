use std::num::NonZeroIsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::thread;

use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use serde::Serialize;
use tracing::{debug, warn};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH,
    PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, ICON_BIG, ICON_SMALL, IDC_ARROW, IsIconic, KillTimer, LoadCursorW,
    MINMAXINFO, PostMessageW, PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
    SW_RESTORE, SW_SHOW, SendMessageW, SetForegroundWindow, SetTimer, ShowWindow, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_GETMINMAXINFO, WM_PAINT, WM_SETICON, WM_SIZE,
    WM_TIMER, WNDCLASSW, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
    WS_THICKFRAME, WS_VISIBLE,
};
use windows::core::w;
use wry::WebViewBuilder;

use crate::tray::TrayEvent;

const PANEL_HTML: &str = include_str!("panel_ui.html");
const WM_PANEL_RESIZED: u32 = WM_APP + 11;
const WM_PANEL_REFRESH: u32 = WM_APP + 12;
const TIMER_REFRESH: usize = 3001;
const REFRESH_MS: u32 = 700;

const PANEL_WIDTH: i32 = 460;
const PANEL_HEIGHT: i32 = 640;
const PANEL_MIN_WIDTH: i32 = 380;
const PANEL_MIN_HEIGHT: i32 = 420;

static OPEN_PANEL: AtomicIsize = AtomicIsize::new(0);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PanelCheck {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PanelSnapshot {
    pub status: String,
    pub party_line: String,
    pub in_room: bool,
    pub auto_accept: bool,
    pub random_skin: bool,
    pub autostart: bool,
    pub update_line: Option<String>,
    pub checks: Vec<PanelCheck>,
}

pub type SnapshotSource = Arc<dyn Fn() -> PanelSnapshot + Send + Sync>;

#[derive(Clone)]
pub struct PanelLinks {
    pub events: Sender<TrayEvent>,
    pub snapshot: SnapshotSource,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PanelLabels {
    lang: &'static str,
    version: &'static str,
    section_options: &'static str,
    section_party: &'static str,
    section_folders: &'static str,
    section_diagnostics: &'static str,
    auto_accept: &'static str,
    random_skin: &'static str,
    random_skin_hint: &'static str,
    autostart: &'static str,
    party_create: &'static str,
    party_join: &'static str,
    party_leave: &'static str,
    open_mods: &'static str,
    open_tools: &'static str,
    open_logs: &'static str,
    about: &'static str,
    quit: &'static str,
    update_download: &'static str,
    mark_problem: &'static str,
    mark_problem_hint: &'static str,
    export_diagnostics: &'static str,
}

impl PanelLabels {
    fn from_text(text: &crate::i18n::Text) -> Self {
        Self {
            lang: text.html_lang,
            version: crate::version::display_version(),
            section_options: text.panel_section_options,
            section_party: text.menu_group_party,
            section_folders: text.menu_group_folders,
            section_diagnostics: text.panel_section_diagnostics,
            auto_accept: text.menu_auto_accept,
            random_skin: text.menu_random_skin,
            random_skin_hint: text.panel_random_skin_hint,
            autostart: text.menu_autostart,
            party_create: text.menu_party_create,
            party_join: text.menu_party_join,
            party_leave: text.menu_party_leave,
            open_mods: text.menu_open_mods,
            open_tools: text.menu_open_tools,
            open_logs: text.menu_open_logs,
            about: text.menu_about,
            quit: text.menu_quit,
            update_download: text.panel_update_download,
            mark_problem: text.panel_mark_problem,
            mark_problem_hint: text.panel_mark_problem_hint,
            export_diagnostics: text.panel_export_diagnostics,
        }
    }
}

#[must_use]
pub fn event_for(message: &str) -> Option<TrayEvent> {
    Some(match message {
        "toggle:autoAccept" => TrayEvent::ToggleAutoAccept,
        "toggle:randomSkin" => TrayEvent::ToggleRandomSkin,
        "toggle:autostart" => TrayEvent::ToggleAutostart,
        "party:create" => TrayEvent::PartyCreate,
        "party:join" => TrayEvent::PartyJoin,
        "party:leave" => TrayEvent::PartyLeave,
        "open:mods" => TrayEvent::OpenMods,
        "open:tools" => TrayEvent::OpenTools,
        "open:logs" => TrayEvent::OpenLogs,
        "about" => TrayEvent::About,
        "quit" => TrayEvent::Quit,
        "open:release" => TrayEvent::OpenRelease,
        "diag:mark" => TrayEvent::MarkProblem,
        "diag:export" => TrayEvent::ExportDiagnostics,
        _ => return None,
    })
}

fn panel_html(labels: &PanelLabels) -> Result<String, serde_json::Error> {
    let json = serde_json::to_string(labels)?.replace("</", "<\\/");
    Ok(PANEL_HTML
        .replace("{{lang}}", labels.lang)
        .replace("{{labels}}", &json))
}

pub fn show_panel(links: PanelLinks) {
    let existing = OPEN_PANEL.load(Ordering::SeqCst);
    if existing != 0 {
        let hwnd = HWND(existing as *mut _);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                // ignore-ok: ShowWindow returns the previous visibility, not a status
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            // ignore-ok: Windows may refuse the foreground; the window is still open and visible
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    thread::spawn(move || run_panel(&links));
}

struct PanelWindowHandle(HWND);

impl HasWindowHandle for PanelWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let non_zero = NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(non_zero));
        unsafe { Ok(WindowHandle::borrow_raw(raw)) }
    }
}

fn client_bounds(hwnd: HWND) -> wry::Rect {
    let mut rect = RECT::default();
    unsafe {
        // ignore-ok: a zeroed rect sizes the WebView at 0x0 until the next WM_SIZE
        let _ = GetClientRect(hwnd, &mut rect);
    }
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(0, 0).into(),
        size: wry::dpi::LogicalSize::new(
            (rect.right - rect.left).max(0) as u32,
            (rect.bottom - rect.top).max(0) as u32,
        )
        .into(),
    }
}

fn run_panel(links: &PanelLinks) {
    let class_name = w!("DekanPanelWindowClass");
    let hicon = crate::welcome::load_dekan_icon().unwrap_or_default();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(panel_wnd_proc),
        lpszClassName: class_name,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hIcon: hicon,
        hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
        ..Default::default()
    };
    unsafe {
        let _ = RegisterClassW(&wc); // ignore-ok: a second registration fails only because the class exists
    }

    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let hwnd = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("Dekan"),
            WS_OVERLAPPED
                | WS_CAPTION
                | WS_SYSMENU
                | WS_THICKFRAME
                | WS_MINIMIZEBOX
                | WS_MAXIMIZEBOX
                | WS_VISIBLE,
            (screen_w - PANEL_WIDTH) / 2,
            (screen_h - PANEL_HEIGHT) / 2,
            PANEL_WIDTH,
            PANEL_HEIGHT,
            None,
            None,
            None,
            None,
        )
    } {
        Ok(hwnd) => hwnd,
        Err(e) => {
            warn!(error = %e, "Control panel: the window could not be created");
            return;
        }
    };
    if OPEN_PANEL
        .compare_exchange(0, hwnd.0 as isize, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        unsafe {
            // ignore-ok: a panel is already open; this duplicate is discarded
            let _ = DestroyWindow(hwnd);
        }
        return;
    }

    if !hicon.is_invalid() {
        unsafe {
            // ignore-ok: WM_SETICON returns the previous icon, not a status
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_BIG as usize),
                LPARAM(hicon.0 as isize),
            );
            // ignore-ok: WM_SETICON returns the previous icon, not a status
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_SMALL as usize),
                LPARAM(hicon.0 as isize),
            );
        }
    }
    let dark: i32 = 1;
    unsafe {
        // ignore-ok: cosmetic; older Windows builds refuse this attribute by design
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as *const _,
            std::mem::size_of::<i32>() as u32,
        );
    }
    crate::paths::ensure_webview2_data_dir();

    let html = match panel_html(&PanelLabels::from_text(crate::i18n::text())) {
        Ok(html) => html,
        Err(e) => {
            warn!(error = %e, "Control panel: labels could not be serialized");
            close_failed(hwnd);
            return;
        }
    };
    let host = PanelWindowHandle(hwnd);
    let hwnd_raw = hwnd.0 as isize;
    let events = links.events.clone();
    let webview = WebViewBuilder::new()
        .with_html(html)
        .with_bounds(client_bounds(hwnd))
        .with_ipc_handler(move |request| {
            let body = request.body().as_str();
            match event_for(body) {
                Some(event) => {
                    if events.send(event).is_err() {
                        warn!(
                            message = body,
                            "Control panel: Dekan is shutting down; the action was not taken"
                        );
                    }
                }
                None if body == "ready" => {}
                None => debug!(message = body, "Control panel: unknown message ignored"),
            }
            unsafe {
                // ignore-ok: our own window; a lost refresh is redone by the next timer tick
                let _ = PostMessageW(
                    HWND(hwnd_raw as *mut _),
                    WM_PANEL_REFRESH,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        })
        .build_as_child(&host);
    let webview = match webview {
        Ok(webview) => webview,
        Err(e) => {
            warn!(error = %e, "Control panel: the WebView could not be created");
            close_failed(hwnd);
            return;
        }
    };

    unsafe {
        // ignore-ok: ShowWindow returns the previous visibility, not a status
        let _ = ShowWindow(hwnd, SW_SHOW);
        // ignore-ok: Windows may refuse the foreground; the window is still shown
        let _ = SetForegroundWindow(hwnd);
        // ignore-ok: without the timer the panel refreshes only when the user acts on it
        let _ = SetTimer(hwnd, TIMER_REFRESH, REFRESH_MS, None);
    }

    let mut last_sent = String::new();
    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                WM_PANEL_RESIZED => {
                    if let Err(e) = webview.set_bounds(client_bounds(hwnd)) {
                        debug!(error = %e, "Control panel: the WebView could not be resized");
                    }
                    continue;
                }
                WM_PANEL_REFRESH => {
                    match serde_json::to_string(&(links.snapshot)()) {
                        Ok(json) if json != last_sent => {
                            let script =
                                format!("window.dekanPanel && window.dekanPanel.render({json});");
                            if let Err(e) = webview.evaluate_script(&script) {
                                debug!(error = %e, "Control panel: the page could not be refreshed");
                            } else {
                                last_sent = json;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            warn!(error = %e, "Control panel: the state could not be serialized")
                        }
                    }
                    continue;
                }
                _ => {}
            }
            // ignore-ok: reports whether a key message was translated; nothing depends on it
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    OPEN_PANEL.store(0, Ordering::SeqCst);
}

fn close_failed(hwnd: HWND) {
    OPEN_PANEL.store(0, Ordering::SeqCst);
    unsafe {
        // ignore-ok: the only failure is an already-gone window, which is the wanted outcome
        let _ = DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn panel_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == TIMER_REFRESH => {
            unsafe {
                // ignore-ok: our own window; a lost refresh is redone by the next tick
                let _ = PostMessageW(hwnd, WM_PANEL_REFRESH, WPARAM(0), LPARAM(0));
            }
            LRESULT(0)
        }
        WM_SIZE => {
            unsafe {
                // ignore-ok: our own window; a lost resize is redone by the next WM_SIZE
                let _ = PostMessageW(hwnd, WM_PANEL_RESIZED, WPARAM(0), LPARAM(0));
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = lparam.0 as *mut MINMAXINFO;
            if !info.is_null() {
                unsafe {
                    (*info).ptMinTrackSize.x = PANEL_MIN_WIDTH;
                    (*info).ptMinTrackSize.y = PANEL_MIN_HEIGHT;
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                // ignore-ok: the timer dies with the window either way
                let _ = KillTimer(hwnd, TIMER_REFRESH);
                // ignore-ok: the only failure is an already-gone window, which is the wanted outcome
                let _ = DestroyWindow(hwnd);
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
                let brush = CreateSolidBrush(COLORREF(0x000C0805));
                FillRect(hdc, &rect, brush);
                // ignore-ok: a failed delete leaks one brush; nothing else depends on it
                let _ = DeleteObject(brush);
                // ignore-ok: EndPaint always succeeds for a PAINTSTRUCT from BeginPaint
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
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
    fn test_every_page_message_maps_to_one_tray_event() {
        let page = PANEL_HTML;
        for message in [
            "toggle:autoAccept",
            "toggle:randomSkin",
            "toggle:autostart",
            "party:create",
            "party:join",
            "party:leave",
            "open:mods",
            "open:tools",
            "open:logs",
            "about",
            "quit",
            "open:release",
            "diag:mark",
            "diag:export",
        ] {
            assert!(event_for(message).is_some(), "{message}");
            assert!(page.contains(message), "the page sends {message}");
        }
        assert_eq!(event_for("rm -rf"), None);
    }

    #[test]
    #[ignore = "writes the rendered control panel for the visual tests into DEKAN_UI_DUMP"]
    fn dump_rendered_panel() {
        let Ok(dir) = std::env::var("DEKAN_UI_DUMP") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("dump dir");
        for (tag, language) in [
            ("tr", Language::Turkish),
            ("en", Language::English),
        ] {
            let html = panel_html(&PanelLabels::from_text(language.text())).expect("labels");
            std::fs::write(dir.join(format!("panel-{tag}.html")), html).expect("panel");
        }
    }

    #[test]
    fn test_every_language_fills_the_page() {
        for language in [Language::Turkish, Language::English] {
            let html = panel_html(&PanelLabels::from_text(language.text())).expect("labels");
            assert!(!html.contains("{{"), "{language:?} left a placeholder");
        }
    }
}
