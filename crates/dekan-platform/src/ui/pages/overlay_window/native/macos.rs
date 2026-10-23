use slint::winit_030::{WinitWindowAccessor, winit};

use crate::client_window::WindowRect;

pub(in super::super) fn prepare(window: &slint::Window, hwnd: isize) {
    window.with_winit_window(|window: &winit::window::Window| {
        window.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
    });
    hide(window, hwnd);
}

pub(in super::super) fn show_at(
    window: &slint::Window,
    _hwnd: isize,
    rect: WindowRect,
) -> Result<(), String> {
    window.set_size(slint::LogicalSize::new(
        rect.width() as f32,
        rect.height() as f32,
    ));
    window
        .with_winit_window(|window: &winit::window::Window| {
            window.set_outer_position(winit::dpi::LogicalPosition::new(rect.left, rect.top));
            window.set_visible(true);
        })
        .ok_or_else(|| "the overlay has no native window".to_owned())
}

pub(in super::super) fn hide(window: &slint::Window, _hwnd: isize) {
    window.with_winit_window(|window: &winit::window::Window| window.set_visible(false));
}
