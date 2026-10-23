use std::sync::OnceLock;
use std::time::{Duration, Instant};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use slint::winit_030::winit;
use tracing::debug;

use crate::error::PlatformError;

const STARTUP_WAIT: Duration = Duration::from_secs(10);
const STARTUP_POLL: Duration = Duration::from_millis(10);

static MAIN_THREAD_UI: OnceLock<Result<(), String>> = OnceLock::new();

pub fn run_interface_on_main_thread() -> Result<(), PlatformError> {
    let selected = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(|attributes| attributes.with_active(false))
        .select()
        .map_err(|e| e.to_string());
    MAIN_THREAD_UI
        .get_or_init(|| selected)
        .clone()
        .map_err(PlatformError::Window)?;
    slint::run_event_loop_until_quit()
        .map_err(|e| PlatformError::Window(format!("the interface event loop stopped: {e}")))
}

pub(super) fn start() -> Result<(), PlatformError> {
    let deadline = Instant::now() + STARTUP_WAIT;
    loop {
        if let Some(ready) = MAIN_THREAD_UI.get() {
            return ready.clone().map_err(PlatformError::Window);
        }
        if Instant::now() >= deadline {
            return Err(PlatformError::Window(
                "the interface did not start on the main thread".into(),
            ));
        }
        std::thread::sleep(STARTUP_POLL);
    }
}

pub(crate) fn hwnd_of(window: &winit::window::Window) -> Option<isize> {
    match window.window_handle() {
        Ok(handle) => match handle.as_raw() {
            RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr() as isize),
            _ => {
                debug!("The window handle is not an AppKit handle");
                None
            }
        },
        Err(e) => {
            debug!(error = %e, "The window handle is unavailable");
            None
        }
    }
}

pub(crate) fn repaint_on_expose<C: ComponentHandle + 'static>(_component: &C, _flip: fn(&C)) {}
