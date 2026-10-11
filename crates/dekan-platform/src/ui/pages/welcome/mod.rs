use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tracing::{debug, warn};

use super::runtime;
use super::views::WelcomeWindow;

const AUTO_DISMISS: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WelcomeContent {
    pub pill: &'static str,
    pub items: Vec<&'static str>,
    pub dismiss: &'static str,
}

pub(crate) fn welcome_content(text: &crate::i18n::Text) -> WelcomeContent {
    WelcomeContent {
        pill: text.welcome_active,
        items: non_empty([
            text.welcome_background,
            text.welcome_author,
            text.welcome_tray_hint,
            text.welcome_quote,
        ]),
        dismiss: text.welcome_dismiss,
    }
}

pub(crate) fn about_content(text: &crate::i18n::Text) -> WelcomeContent {
    WelcomeContent {
        pill: text.about_title,
        items: non_empty([
            text.about_educational,
            text.welcome_author,
            text.about_quote,
        ]),
        dismiss: text.about_dismiss,
    }
}

fn non_empty<const N: usize>(items: [&'static str; N]) -> Vec<&'static str> {
    items.into_iter().filter(|item| !item.is_empty()).collect()
}

pub fn show_welcome_window() {
    show(welcome_content(crate::i18n::text()), Some(AUTO_DISMISS));
}

pub fn show_about_window() {
    show(about_content(crate::i18n::text()), None);
}

fn show(content: WelcomeContent, auto_dismiss: Option<Duration>) {
    if let Err(e) = runtime::run_on_ui(move || open(&content, auto_dismiss)) {
        warn!(error = %e, "The welcome window could not be opened");
    }
}

pub(crate) fn fill(window: &WelcomeWindow, content: &WelcomeContent) {
    window.set_version(crate::version::display_version().into());
    window.set_pill(content.pill.into());
    let items: Vec<SharedString> = content.items.iter().map(|item| (*item).into()).collect();
    window.set_items(ModelRc::new(VecModel::from(items)));
    window.set_dismiss_label(content.dismiss.into());
}

fn open(content: &WelcomeContent, auto_dismiss: Option<Duration>) {
    let window = match WelcomeWindow::new() {
        Ok(window) => window,
        Err(e) => {
            warn!(error = %e, "The welcome window could not be created");
            return;
        }
    };
    fill(&window, content);
    runtime::repaint_on_expose(&window, |w| w.set_expose_flip(!w.get_expose_flip()));
    let weak = window.as_weak();
    window.on_dismiss(move || {
        if let Some(window) = weak.upgrade() {
            close(&window);
        }
    });
    if let Err(e) = window.show() {
        warn!(error = %e, "The welcome window could not be shown");
        return;
    }
    if let Some(after) = auto_dismiss {
        let weak = window.as_weak();
        slint::Timer::single_shot(after, move || {
            if let Some(window) = weak.upgrade() {
                close(&window);
            }
        });
    }
}

fn close(window: &WelcomeWindow) {
    if let Err(e) = window.hide() {
        debug!(error = %e, "The welcome window was already closed");
    }
}

#[cfg(test)]
mod tests;
