use std::sync::mpsc;

use slint::ComponentHandle;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, warn};

use super::runtime;
use super::views::StarGateWindow;
use crate::error::PlatformError;

pub const REPOSITORY_URL: &str = "https://github.com/chrisssst/Dekan";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateEvent {
    SignIn,
    Recheck,
    Quit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateView {
    pub message: String,
    pub is_error: bool,
    pub code: String,
    pub busy: bool,
    pub can_recheck: bool,
}

pub struct StarGateDialog {
    window: slint::Weak<StarGateWindow>,
}

impl StarGateDialog {
    pub fn open(events: UnboundedSender<GateEvent>) -> Result<Self, PlatformError> {
        let (created_tx, created_rx) = mpsc::channel();
        runtime::run_on_ui(move || {
            let created = create(events).map(|window| window.as_weak());
            if created_tx.send(created).is_err() {
                debug!("Nobody is waiting for the star gate window any more");
            }
        })?;
        let window = created_rx
            .recv()
            .map_err(|_| PlatformError::Window("the star gate window was never created".into()))?
            .map_err(|e| PlatformError::Window(format!("the star gate window failed: {e}")))?;
        Ok(Self { window })
    }

    pub fn show(&self, view: GateView) {
        if let Err(e) = self.window.upgrade_in_event_loop(move |w| apply(&w, &view)) {
            warn!(error = %e, "The star gate window could not be updated");
        }
    }

    pub fn close(&self) {
        let closed = self.window.upgrade_in_event_loop(|w| {
            if let Err(e) = w.hide() {
                debug!(error = %e, "The star gate window was already hidden");
            }
        });
        if let Err(e) = closed {
            debug!(error = %e, "The star gate window was already gone");
        }
    }
}

fn create(events: UnboundedSender<GateEvent>) -> Result<StarGateWindow, slint::PlatformError> {
    let window = StarGateWindow::new()?;
    fill(&window, crate::i18n::text());
    super::themes::follow(&window, |w, mode| {
        w.global::<super::views::Theme>().set_mode(mode)
    });
    runtime::repaint_on_expose(&window, |w| w.set_expose_flip(!w.get_expose_flip()));
    wire(&window, &events);
    window.show()?;
    runtime::when_created(&window, |_, hwnd| {
        crate::client_window::take_foreground(hwnd);
    });
    Ok(window)
}

pub(crate) fn fill(window: &StarGateWindow, text: &crate::i18n::Text) {
    window.set_version(crate::version::display_version().into());
    window.set_heading(text.star_gate_title.into());
    window.set_intro(text.star_gate_intro.into());
    window.set_sign_in_label(text.star_gate_sign_in.into());
    window.set_open_repo_label(text.star_gate_open_repo.into());
    window.set_recheck_label(text.star_gate_recheck.into());
    window.set_quit_label(text.star_gate_quit.into());
    window.set_code_hint(text.star_gate_code_hint.into());
}

pub(crate) fn apply(window: &StarGateWindow, view: &GateView) {
    window.set_message(view.message.as_str().into());
    window.set_is_error(view.is_error);
    window.set_code(view.code.as_str().into());
    window.set_busy(view.busy);
    window.set_can_recheck(view.can_recheck);
}

fn emit(events: &UnboundedSender<GateEvent>, event: GateEvent) {
    if events.send(event).is_err() {
        debug!(
            ?event,
            "Nobody is listening to the star gate window any more"
        );
    }
}

pub(crate) fn wire(window: &StarGateWindow, events: &UnboundedSender<GateEvent>) {
    let tx = events.clone();
    window.on_sign_in(move || emit(&tx, GateEvent::SignIn));
    let tx = events.clone();
    window.on_recheck(move || emit(&tx, GateEvent::Recheck));
    let tx = events.clone();
    window.on_quit(move || emit(&tx, GateEvent::Quit));
    window.on_open_repo(|| {
        if let Err(e) = crate::shell::open_web_page(REPOSITORY_URL) {
            warn!(error = %e, "The repository page could not be opened");
        }
    });
    let tx = events.clone();
    window.window().on_close_requested(move || {
        emit(&tx, GateEvent::Quit);
        slint::CloseRequestResponse::HideWindow
    });
}

#[cfg(test)]
mod tests;
