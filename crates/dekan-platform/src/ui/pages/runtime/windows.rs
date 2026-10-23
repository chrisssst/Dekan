use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use slint::winit_030::winit;
use tracing::{debug, error};
use windows::Win32::Graphics::Gdi::GetUpdateRect;
use windows::Win32::UI::WindowsAndMessaging::{MSG, WM_PAINT};

use crate::error::PlatformError;

static UI_THREAD: OnceLock<Result<(), String>> = OnceLock::new();

thread_local! {
    static REPAINTERS: RefCell<Vec<Box<dyn Fn() -> bool>>> = const { RefCell::new(Vec::new()) };
    static REPAINT_PENDING: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn start() -> Result<(), PlatformError> {
    UI_THREAD
        .get_or_init(|| {
            let (ready_tx, ready_rx) = mpsc::channel();
            std::thread::Builder::new()
                .name("dekan-ui".into())
                .spawn(move || {
                    let mut events = winit::event_loop::EventLoop::<slint::winit_030::SlintEvent>::with_user_event();
                    {
                        use winit::platform::windows::EventLoopBuilderExtWindows;
                        events.with_msg_hook(|msg| {
                            repaint_if_exposed(msg.cast::<MSG>());
                            false
                        });
                    }
                    let selected = slint::BackendSelector::new()
                        .backend_name("winit".into())
                        .with_winit_event_loop_builder(events)
                        .renderer_name("software".into())
                        .with_winit_window_attributes_hook(|attributes| {
                            attributes.with_active(false)
                        })
                        .select();
                    let failed = selected.is_err();
                    if ready_tx.send(selected.map_err(|e| e.to_string())).is_err() || failed {
                        return;
                    }
                    if let Err(e) = slint::run_event_loop_until_quit() {
                        error!(error = %e, "The interface event loop stopped");
                    }
                })
                .map_err(|e| format!("could not start the interface thread: {e}"))?;
            ready_rx
                .recv()
                .map_err(|_| "the interface thread ended during startup".to_owned())?
        })
        .clone()
        .map_err(PlatformError::Window)
}

pub(crate) fn hwnd_of(window: &winit::window::Window) -> Option<isize> {
    match window.window_handle() {
        Ok(handle) => match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => {
                debug!("The window handle is not a Win32 handle");
                None
            }
        },
        Err(e) => {
            debug!(error = %e, "The window handle is unavailable");
            None
        }
    }
}

pub(crate) fn repaint_on_expose<C: ComponentHandle + 'static>(component: &C, flip: fn(&C)) {
    let weak = component.as_weak();
    REPAINTERS.with_borrow_mut(|repainters| {
        repainters.push(Box::new(move || match weak.upgrade() {
            Some(component) => {
                flip(&component);
                true
            }
            None => false,
        }));
    });
}

fn repaint_if_exposed(msg: *const MSG) {
    let Some(msg) = (unsafe { msg.as_ref() }) else {
        return;
    };
    let exposed =
        msg.message == WM_PAINT && unsafe { GetUpdateRect(msg.hwnd, None, false) }.as_bool();
    if !exposed || REPAINT_PENDING.replace(true) {
        return;
    }
    slint::Timer::single_shot(Duration::ZERO, || {
        REPAINT_PENDING.set(false);
        REPAINTERS.with_borrow_mut(|repainters| repainters.retain(|repaint| repaint()));
    });
}
