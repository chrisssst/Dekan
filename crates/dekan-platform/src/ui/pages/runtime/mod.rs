use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use tracing::{debug, warn};

use crate::error::PlatformError;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::run_interface_on_main_thread;
#[cfg(target_os = "macos")]
use macos::start;
#[cfg(target_os = "macos")]
pub(crate) use macos::{hwnd_of, repaint_on_expose};
#[cfg(windows)]
use windows::start;
#[cfg(windows)]
pub(crate) use windows::{hwnd_of, repaint_on_expose};

pub(crate) fn run_on_ui(job: impl FnOnce() + Send + 'static) -> Result<(), PlatformError> {
    start()?;
    slint::invoke_from_event_loop(job)
        .map_err(|e| PlatformError::Window(format!("the interface thread is gone: {e}")))
}

pub(crate) fn hwnd_now(window: &slint::Window) -> Option<isize> {
    window.with_winit_window(hwnd_of).flatten()
}

pub(crate) fn image(bytes: &[u8]) -> Option<slint::Image> {
    slint::Image::load_from_data(bytes, None).ok()
}

pub(crate) fn when_created<C: ComponentHandle + 'static>(
    component: &C,
    then: impl FnOnce(&C, isize) + 'static,
) {
    let weak = component.as_weak();
    let spawned = slint::spawn_local(async move {
        let Some(component) = weak.upgrade() else {
            return;
        };
        match component.window().winit_window().await {
            Ok(window) => match hwnd_of(&window) {
                Some(hwnd) => then(&component, hwnd),
                None => warn!("A window was created but its native handle could not be read"),
            },
            Err(e) => warn!(error = %e, "A window closed before it was created"),
        }
    });
    if let Err(e) = spawned {
        debug!(error = %e, "Could not wait for a window to be created");
    }
}
