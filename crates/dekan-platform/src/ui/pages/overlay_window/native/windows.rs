use slint::winit_030::{WinitWindowAccessor, winit};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetWindowPos, ShowWindow,
};

use crate::client_window::WindowRect;

pub(in super::super) fn prepare(window: &slint::Window, hwnd: isize) {
    window.with_winit_window(|window: &winit::window::Window| {
        use winit::platform::windows::WindowExtWindows;
        window.set_skip_taskbar(true);
    });
    hide(window, hwnd);
}

pub(in super::super) fn show_at(
    _window: &slint::Window,
    hwnd: isize,
    rect: WindowRect,
) -> Result<(), String> {
    let target = HWND(hwnd as *mut _);
    unsafe {
        let placed = SetWindowPos(
            target,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            SWP_NOACTIVATE,
        )
        .map_err(|e| e.to_string());
        let _ = ShowWindow(target, SW_SHOWNOACTIVATE); // ignore-ok: returns the previous visibility, not an error
        placed
    }
}

pub(in super::super) fn hide(_window: &slint::Window, hwnd: isize) {
    unsafe {
        let _ = ShowWindow(HWND(hwnd as *mut _), SW_HIDE); // ignore-ok: returns the previous visibility, not an error
    }
}
