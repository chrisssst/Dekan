use tracing::debug;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, FindWindowW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SetForegroundWindow,
};
use windows::core::w;

use super::{ClientWindowState, WindowRect, overlay_placement_on};

pub const LEAGUE_CLIENT_CLASS: &str = "RCLIENT";

fn from_win32(r: RECT) -> WindowRect {
    WindowRect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

#[must_use]
pub fn client_window_state() -> ClientWindowState {
    unsafe {
        let hwnd = match FindWindowW(w!("RCLIENT"), None) {
            Ok(hwnd) if !hwnd.is_invalid() => hwnd,
            _ => return ClientWindowState::Absent,
        };

        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return ClientWindowState::Hidden;
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return ClientWindowState::Hidden;
        }

        let rect = from_win32(rect);
        if rect.is_on_screen() {
            ClientWindowState::Visible(rect)
        } else {
            ClientWindowState::Hidden
        }
    }
}

#[must_use]
pub fn find_client_hwnd() -> Option<HWND> {
    unsafe {
        match FindWindowW(w!("RCLIENT"), None) {
            Ok(hwnd) if !hwnd.is_invalid() => Some(hwnd),
            _ => None,
        }
    }
}

pub(crate) fn take_foreground(hwnd: isize) {
    switch_foreground(HWND(hwnd as *mut _));
}

pub(crate) fn return_foreground_to_client() {
    if let Some(client) = find_client_hwnd() {
        switch_foreground(client);
    }
}

fn switch_foreground(target: HWND) {
    unsafe {
        let foreground_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let current_thread = GetCurrentThreadId();
        let attach = foreground_thread != 0 && foreground_thread != current_thread;
        if attach && !AttachThreadInput(foreground_thread, current_thread, true).as_bool() {
            debug!("Thread input could not be attached; the foreground request may be refused");
        }
        if let Err(e) = BringWindowToTop(target) {
            debug!(error = %e, "Windows refused to raise the window");
        }
        if !SetForegroundWindow(target).as_bool() {
            debug!("Windows refused the foreground; the window stays usable by click");
        }
        if attach {
            let _ = AttachThreadInput(foreground_thread, current_thread, false); // ignore-ok: the attachment ends with the thread anyway
        }
    }
}

#[must_use]
pub fn get_monitor_work_area(hwnd: HWND) -> Option<WindowRect> {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if monitor.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let rect = from_win32(info.rcWork);
            if rect.is_on_screen() {
                Some(rect)
            } else {
                None
            }
        } else {
            None
        }
    }
}

#[must_use]
pub fn overlay_placement(client: WindowRect, width: i32, height: i32, padding: i32) -> WindowRect {
    let monitor = find_client_hwnd().and_then(get_monitor_work_area);
    overlay_placement_on(client, monitor, width, height, padding)
}
