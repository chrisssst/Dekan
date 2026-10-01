use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use std::num::NonZeroIsize;

use dekan_core::overlay::OverlayCommand;
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::{debug, info, warn};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, CreateSolidBrush, SetWindowRgn};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_TOPMOST, MSG,
    PostMessageW, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetForegroundWindow,
    SetWindowPos, ShowWindow, TranslateMessage, WM_APP, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP, WS_THICKFRAME,
};
use windows::core::w;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebViewBuilder};

const OVERLAY_HTML: &str = include_str!("overlay_ui.html");

use crate::client_window::{
    ClientWindowState, WindowRect, client_window_state, overlay_placement, overlay_placement_on,
};
use crate::error::PlatformError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WindowControl {
    Focus,
    Blur,
    Drag,
    Resize,
    Hide,
}

impl WindowControl {
    fn try_parse(payload: &str) -> Option<Self> {
        serde_json::from_str(payload).ok()
    }
}

pub const OVERLAY_WIDTH: i32 = 360;

pub const OVERLAY_HEIGHT: i32 = 520;

pub const OVERLAY_PADDING: i32 = 16;

pub const OVERLAY_CORNER_RADIUS: i32 = 14;

pub const OVERLAY_MIN_WIDTH: i32 = 320;

pub const OVERLAY_MIN_HEIGHT: i32 = 380;

/// Size the user dragged the overlay to; placement keeps it across client moves.
static OVERLAY_SIZE: (AtomicI32, AtomicI32) = (
    AtomicI32::new(OVERLAY_WIDTH),
    AtomicI32::new(OVERLAY_HEIGHT),
);

#[must_use]
pub fn overlay_size() -> (i32, i32) {
    (
        OVERLAY_SIZE.0.load(Ordering::Relaxed),
        OVERLAY_SIZE.1.load(Ordering::Relaxed),
    )
}

const WM_OVERLAY_RESIZED: u32 = WM_APP + 6;

const WM_OVERLAY_SHOW: u32 = WM_APP + 1;

const WM_OVERLAY_HIDE: u32 = WM_APP + 2;

const WM_OVERLAY_QUIT: u32 = WM_APP + 3;

const WM_OVERLAY_SCRIPT: u32 = WM_APP + 4;

const WM_OVERLAY_FOCUS: u32 = WM_APP + 5;

fn pack_point(x: i32, y: i32) -> isize {
    let x = x.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as isize;
    let y = y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as isize;
    (x & 0xFFFF) | ((y & 0xFFFF) << 16)
}

fn unpack_point(packed: isize) -> (i32, i32) {
    let x = (packed & 0xFFFF) as u16 as i16 as i32;
    let y = ((packed >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

#[derive(Clone)]
pub struct OverlayController {
    hwnd: isize,
    alive: Arc<AtomicBool>,

    pending_scripts: Arc<Mutex<Vec<String>>>,
}

impl OverlayController {
    #[must_use]
    pub fn window_handle(&self) -> isize {
        self.hwnd
    }

    pub fn show_at(&self, rect: WindowRect) {
        if !self.alive.load(Ordering::SeqCst) {
            return;
        }

        unsafe {
            if let Err(e) = PostMessageW(
                HWND(self.hwnd as *mut _),
                WM_OVERLAY_SHOW,
                WPARAM(pack_point(rect.left, rect.top) as usize),
                LPARAM(pack_point(rect.width(), rect.height())),
            ) {
                warn!(error = %e, "Could not post show request to the overlay window");
            }
        }
    }

    pub fn hide(&self) {
        if !self.alive.load(Ordering::SeqCst) {
            return;
        }

        unsafe {
            if let Err(e) = PostMessageW(
                HWND(self.hwnd as *mut _),
                WM_OVERLAY_HIDE,
                WPARAM(0),
                LPARAM(0),
            ) {
                warn!(error = %e, "Could not post hide request to the overlay window");
            }
        }
    }

    pub fn set_catalog(&self, catalog_json: String) {
        self.eval_script(format!("window.dekanOverlay.setCatalog({catalog_json});"));
    }

    pub fn eval_script(&self, script: String) {
        if !self.alive.load(Ordering::SeqCst) {
            return;
        }

        match self.pending_scripts.lock() {
            Ok(mut queue) => queue.push(script),
            Err(e) => {
                warn!(error = %e, "Overlay script queue is poisoned; dropping this update");
                return;
            }
        }

        unsafe {
            if let Err(e) = PostMessageW(
                HWND(self.hwnd as *mut _),
                WM_OVERLAY_SCRIPT,
                WPARAM(0),
                LPARAM(0),
            ) {
                warn!(error = %e, "Could not notify the overlay about queued work");
            }
        }
    }

    pub fn shutdown(&self) {
        if !self.alive.swap(false, Ordering::SeqCst) {
            return;
        }

        unsafe {
            // ignore-ok: the target window is our own and `alive` was checked; a failure means it is already closed
            let _ = PostMessageW(
                HWND(self.hwnd as *mut _),
                WM_OVERLAY_QUIT,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

pub struct OverlayWindow {
    controller: OverlayController,
}

impl OverlayWindow {
    pub fn spawn() -> Result<(Self, UnboundedReceiver<OverlayCommand>), PlatformError> {
        let (ready_tx, ready_rx) = channel::<Result<isize, String>>();
        let (command_tx, command_rx) = unbounded_channel::<OverlayCommand>();
        let alive = Arc::new(AtomicBool::new(true));
        let pending_scripts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        {
            let alive = alive.clone();
            let pending_scripts = pending_scripts.clone();
            thread::spawn(move || {
                run_overlay_message_loop(ready_tx, alive, pending_scripts, command_tx)
            });
        }

        let hwnd = ready_rx
            .recv()
            .map_err(|_| PlatformError::Window("overlay window thread died at startup".into()))?
            .map_err(PlatformError::Window)?;

        info!("Overlay window and WebView surface created");
        Ok((
            Self {
                controller: OverlayController {
                    hwnd,
                    alive,
                    pending_scripts,
                },
            },
            command_rx,
        ))
    }

    #[must_use]
    pub fn controller(&self) -> OverlayController {
        self.controller.clone()
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        self.controller.shutdown();
    }
}

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

fn run_overlay_message_loop(
    ready_tx: Sender<Result<isize, String>>,
    alive: Arc<AtomicBool>,
    pending_scripts: Arc<Mutex<Vec<String>>>,
    command_tx: UnboundedSender<OverlayCommand>,
) {
    let class_name = w!("DekanOverlayWindowClass");

    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(overlay_wnd_proc),
            hInstance: Default::default(),
            lpszClassName: class_name,
            hbrBackground: CreateSolidBrush(COLORREF(0x0014_0F0B)),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("Dekan"),
            // WS_THICKFRAME only makes the native size loop available (started from the UI's
            // grip); WM_NCCALCSIZE hands the whole window to the client so no frame is drawn.
            WS_POPUP | WS_THICKFRAME,
            0,
            0,
            OVERLAY_WIDTH,
            OVERLAY_HEIGHT,
            None,
            None,
            None,
            None,
        )
    };

    let hwnd = match hwnd {
        Ok(hwnd) if !hwnd.is_invalid() => hwnd,
        other => {
            warn!(result = ?other, "Failed to create the overlay window");
            let _ = ready_tx.send(Err(format!("overlay window creation failed: {other:?}"))); // ignore-ok: nobody is waiting any more; the thread tears itself down below
            alive.store(false, Ordering::SeqCst);
            return;
        }
    };

    let host = OverlayWindowHandle(hwnd);

    crate::paths::ensure_webview2_data_dir();

    let hwnd_raw = hwnd.0 as isize;
    let html = OVERLAY_HTML.replace("{{version}}", crate::version::display_version());
    let webview = match WebViewBuilder::new()
        .with_html(html)
        .with_ipc_handler(move |request| {
            let payload = request.body();

            match WindowControl::try_parse(payload) {
                Some(WindowControl::Focus) => {
                    unsafe {
                        use windows::Win32::System::Threading::{
                            AttachThreadInput, GetCurrentThreadId,
                        };
                        use windows::Win32::UI::WindowsAndMessaging::{
                            BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId,
                        };

                        let target_hwnd = HWND(hwnd_raw as *mut _);
                        let foreground_hwnd = GetForegroundWindow();
                        let foreground_thread = GetWindowThreadProcessId(foreground_hwnd, None);
                        let current_thread = GetCurrentThreadId();

                        // Only the foreground is taken here. Keyboard focus must land on the
                        // WebView2 child, not on this host: `SetFocus(host)` pulled it out of the
                        // page and the search box stopped receiving keystrokes.
                        if foreground_thread != 0 && foreground_thread != current_thread {
                            let _ = AttachThreadInput(foreground_thread, current_thread, true); // ignore-ok: best-effort thread input attachment
                            let _ = BringWindowToTop(target_hwnd); // ignore-ok: brings window to top of z-order
                            let _ = SetForegroundWindow(target_hwnd); // ignore-ok: transfer foreground focus
                            let _ = AttachThreadInput(foreground_thread, current_thread, false); // ignore-ok: detach thread input after transfer
                        } else {
                            let _ = BringWindowToTop(target_hwnd); // ignore-ok: brings window to top of z-order
                            let _ = SetForegroundWindow(target_hwnd); // ignore-ok: best-effort focus grant
                        }
                        // ignore-ok: our own window; a failure means the loop already ended
                        let _ = PostMessageW(target_hwnd, WM_OVERLAY_FOCUS, WPARAM(0), LPARAM(0));
                    }
                    return;
                }
                Some(WindowControl::Blur) => {
                    if let Some(client) = crate::client_window::find_client_hwnd() {
                        unsafe {
                            use windows::Win32::System::Threading::{
                                AttachThreadInput, GetCurrentThreadId,
                            };
                            use windows::Win32::UI::WindowsAndMessaging::{
                                GetForegroundWindow, GetWindowThreadProcessId,
                            };

                            let foreground_hwnd = GetForegroundWindow();
                            let foreground_thread = GetWindowThreadProcessId(foreground_hwnd, None);
                            let current_thread = GetCurrentThreadId();

                            if foreground_thread != 0 && foreground_thread != current_thread {
                                let _ = AttachThreadInput(foreground_thread, current_thread, true); // ignore-ok: best-effort thread input attachment
                                let _ = SetForegroundWindow(client); // ignore-ok: restore focus to client
                                let _ = AttachThreadInput(foreground_thread, current_thread, false); // ignore-ok: detach thread input after transfer
                            } else {
                                let _ = SetForegroundWindow(client); // ignore-ok: restore focus to client
                            }
                        }
                    }
                    return;
                }
                Some(WindowControl::Drag) => {
                    unsafe {
                        use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
                        use windows::Win32::UI::WindowsAndMessaging::{
                            HTCAPTION, SendMessageW, WM_NCLBUTTONDOWN,
                        };

                        // ignore-ok: initiating window drag via Win32 non-client message
                        let _ = ReleaseCapture();

                        // ignore-ok: non-client click message forwarded to start system window drag
                        let _ = SendMessageW(
                            HWND(hwnd_raw as *mut _),
                            WM_NCLBUTTONDOWN,
                            WPARAM(HTCAPTION as usize),
                            LPARAM(0),
                        );
                    }
                    return;
                }
                Some(WindowControl::Resize) => {
                    unsafe {
                        use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
                        use windows::Win32::UI::WindowsAndMessaging::{
                            HTBOTTOMRIGHT, SendMessageW, WM_NCLBUTTONDOWN,
                        };

                        // ignore-ok: the WebView holds the capture; releasing it lets the size loop take the mouse
                        let _ = ReleaseCapture();

                        // ignore-ok: non-client click on the corner starts the system resize loop
                        let _ = SendMessageW(
                            HWND(hwnd_raw as *mut _),
                            WM_NCLBUTTONDOWN,
                            WPARAM(HTBOTTOMRIGHT as usize),
                            LPARAM(0),
                        );
                    }
                    return;
                }
                Some(WindowControl::Hide) => {
                    unsafe {
                        // ignore-ok: user requested close/hide via overlay titlebar button
                        let _ = ShowWindow(HWND(hwnd_raw as *mut _), SW_HIDE);
                    }
                    return;
                }
                None => {}
            }

            match OverlayCommand::parse(payload) {
                Ok(command) => {
                    // Hovering chromas sends these continuously; they are not state transitions.
                    if matches!(command, OverlayCommand::ChromaPreview { .. }) {
                        debug!(?command, "Overlay UI command received");
                    } else {
                        info!(?command, "Overlay UI command received");
                    }
                    if command_tx.send(command).is_err() {
                        debug!("Nobody is listening for overlay commands any more");
                    }
                }

                Err(e) => {
                    warn!(error = %e, payload = %payload, "Unreadable message from the overlay UI");
                }
            }
        })
        .with_transparent(false)
        .with_bounds(Rect {
            position: LogicalPosition::new(0, 0).into(),
            size: LogicalSize::new(OVERLAY_WIDTH, OVERLAY_HEIGHT).into(),
        })
        .build_as_child(&host)
    {
        Ok(webview) => webview,
        Err(e) => {
            warn!(error = %e, "Could not create the WebView2 overlay surface");
            let _ = ready_tx.send(Err(format!("WebView2 surface unavailable: {e}"))); // ignore-ok: nobody is waiting any more; the thread tears itself down below
            alive.store(false, Ordering::SeqCst);
            return;
        }
    };

    apply_rounded_region(hwnd, OVERLAY_WIDTH, OVERLAY_HEIGHT);

    if ready_tx.send(Ok(hwnd.0 as isize)).is_err() {
        alive.store(false, Ordering::SeqCst);
        return;
    }

    let mut msg = MSG::default();

    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                WM_OVERLAY_QUIT => break,
                WM_OVERLAY_SHOW => {
                    let (x, y) = unpack_point(msg.wParam.0 as isize);
                    let (w, h) = unpack_point(msg.lParam.0);
                    let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE); // ignore-ok: a refused reposition is retried by the next tracking tick, 200 ms later
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE); // ignore-ok: returns the previous visibility, not an error
                    if let Err(e) = webview.set_bounds(Rect {
                        position: LogicalPosition::new(0, 0).into(),
                        size: LogicalSize::new(w, h).into(),
                    }) {
                        debug!(error = %e, "Could not resize the overlay WebView");
                    }
                }
                WM_OVERLAY_HIDE => {
                    let _ = ShowWindow(hwnd, SW_HIDE); // ignore-ok: returns the previous visibility, not an error
                }
                WM_OVERLAY_RESIZED => {
                    let (w, h) = overlay_size();
                    apply_rounded_region(hwnd, w, h);
                    if let Err(e) = webview.set_bounds(Rect {
                        position: LogicalPosition::new(0, 0).into(),
                        size: LogicalSize::new(w, h).into(),
                    }) {
                        debug!(error = %e, "Could not resize the overlay WebView");
                    }
                }
                WM_OVERLAY_FOCUS => {
                    if let Err(e) = webview.focus() {
                        debug!(error = %e, "Could not move keyboard focus into the overlay WebView");
                    }
                }
                WM_OVERLAY_SCRIPT => {
                    let queued: Vec<String> = pending_scripts
                        .lock()
                        .map(|mut queue| std::mem::take(&mut *queue))
                        .unwrap_or_default();
                    for script in queued {
                        if let Err(e) = webview.evaluate_script(&script) {
                            warn!(error = %e, "Could not run a script in the overlay UI");
                        }
                    }
                }
                _ => {
                    let _ = TranslateMessage(&msg); // ignore-ok: returns whether a key event was translated; this pump forwards either way
                    DispatchMessageW(&msg);
                }
            }
        }
    }

    drop(webview);
    alive.store(false, Ordering::SeqCst);
    debug!("Overlay message loop finished");
}

fn apply_rounded_region(hwnd: HWND, width: i32, height: i32) {
    unsafe {
        let region = CreateRoundRectRgn(
            0,
            0,
            width + 1,
            height + 1,
            OVERLAY_CORNER_RADIUS,
            OVERLAY_CORNER_RADIUS,
        );
        if SetWindowRgn(hwnd, region, true) == 0 {
            warn!("Could not apply rounded-corner region to the overlay window");
        }
    }
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        MINMAXINFO, WM_GETMINMAXINFO, WM_NCCALCSIZE, WM_SIZE,
    };

    match msg {
        // The whole window is client area: no frame from WS_THICKFRAME is ever painted.
        WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
        WM_GETMINMAXINFO => {
            let info = lparam.0 as *mut MINMAXINFO;
            if !info.is_null() {
                // SAFETY: Windows passes a valid MINMAXINFO for this message.
                unsafe {
                    (*info).ptMinTrackSize.x = OVERLAY_MIN_WIDTH;
                    (*info).ptMinTrackSize.y = OVERLAY_MIN_HEIGHT;
                }
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let (w, h) = unpack_point(lparam.0);
            if w > 0 && h > 0 {
                OVERLAY_SIZE.0.store(w, Ordering::Relaxed);
                OVERLAY_SIZE.1.store(h, Ordering::Relaxed);
                unsafe {
                    // ignore-ok: our own window; a lost message is redone by the next WM_SIZE
                    let _ = PostMessageW(hwnd, WM_OVERLAY_RESIZED, WPARAM(0), LPARAM(0));
                }
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

struct OverlayWindowHandle(HWND);

impl HasWindowHandle for OverlayWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?;
        let handle = Win32WindowHandle::new(raw);

        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

#[cfg(test)]
#[path = "overlay_window_tests.rs"]
mod tests;
