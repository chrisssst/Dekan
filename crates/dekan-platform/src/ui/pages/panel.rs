use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, VecModel};
use tracing::{debug, warn};

use super::runtime;
use super::views::{ControlPanel, PanelAction, PanelCheckRow, PanelLabels};
use crate::tray::TrayEvent;

const REFRESH: Duration = Duration::from_millis(700);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelCheck {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSnapshot {
    pub status: String,
    pub party_line: String,
    pub in_room: bool,
    pub auto_accept: bool,
    pub random_skin: bool,
    pub light_loading: bool,
    pub autostart: bool,
    pub update_line: Option<String>,
    pub ltk_line: Option<String>,
    pub ltk_download: Option<String>,
    pub checks: Vec<PanelCheck>,
}

pub type SnapshotSource = Arc<dyn Fn() -> PanelSnapshot + Send + Sync>;

#[derive(Clone)]
pub struct PanelLinks {
    pub events: tokio::sync::mpsc::UnboundedSender<TrayEvent>,
    pub snapshot: SnapshotSource,
}

struct OpenPanel {
    window: ControlPanel,
    _refresh: slint::Timer,
}

thread_local! {
    static OPEN_PANEL: RefCell<Option<OpenPanel>> = const { RefCell::new(None) };
}

#[must_use]
pub(crate) fn event_for(action: PanelAction) -> TrayEvent {
    match action {
        PanelAction::ToggleRandomSkin => TrayEvent::ToggleRandomSkin,
        PanelAction::ToggleLightLoading => TrayEvent::ToggleLightLoading,
        PanelAction::ToggleAutoAccept => TrayEvent::ToggleAutoAccept,
        PanelAction::ToggleAutostart => TrayEvent::ToggleAutostart,
        PanelAction::PartyCreate => TrayEvent::PartyCreate,
        PanelAction::PartyJoin => TrayEvent::PartyJoin,
        PanelAction::PartyLeave => TrayEvent::PartyLeave,
        PanelAction::OpenMods => TrayEvent::OpenMods,
        PanelAction::OpenTools => TrayEvent::OpenTools,
        PanelAction::OpenLogs => TrayEvent::OpenLogs,
        PanelAction::OpenRelease => TrayEvent::OpenRelease,
        PanelAction::InstallInjector => TrayEvent::InstallInjector,
        PanelAction::MarkProblem => TrayEvent::MarkProblem,
        PanelAction::ExportDiagnostics => TrayEvent::ExportDiagnostics,
        PanelAction::About => TrayEvent::About,
        PanelAction::Quit => TrayEvent::Quit,
    }
}

pub(crate) fn labels(text: &crate::i18n::Text) -> PanelLabels {
    PanelLabels {
        version: crate::version::display_version().into(),
        section_options: text.panel_section_options.into(),
        section_party: text.menu_group_party.into(),
        section_folders: text.menu_group_folders.into(),
        section_diagnostics: text.panel_section_diagnostics.into(),
        random_skin: text.menu_random_skin.into(),
        random_skin_hint: text.panel_random_skin_hint.into(),
        light_loading: text.menu_light_loading.into(),
        light_loading_hint: text.panel_light_loading_hint.into(),
        auto_accept: text.menu_auto_accept.into(),
        autostart: text.menu_autostart.into(),
        party_create: text.menu_party_create.into(),
        party_join: text.menu_party_join.into(),
        party_leave: text.menu_party_leave.into(),
        open_mods: text.folder_mods.into(),
        open_tools: text.folder_tools.into(),
        open_logs: text.folder_logs.into(),
        about: text.menu_about.into(),
        quit: text.menu_quit.into(),
        update_download: text.panel_update_download.into(),
        mark_problem: text.panel_mark_problem.into(),
        mark_problem_hint: text.panel_mark_problem_hint.into(),
        export_diagnostics: text.panel_export_diagnostics.into(),
    }
}

pub(crate) fn follow_language(
    window: &ControlPanel,
    shown: &std::cell::Cell<Option<crate::i18n::Language>>,
    active: crate::i18n::Language,
) {
    if shown.replace(Some(active)) != Some(active) {
        window.set_labels(labels(active.text()));
    }
}

pub(crate) fn render(window: &ControlPanel, state: &PanelSnapshot) {
    window.set_status(state.status.as_str().into());
    window.set_party_line(state.party_line.as_str().into());
    window.set_in_room(state.in_room);
    window.set_random_skin(state.random_skin);
    window.set_light_loading(state.light_loading);
    window.set_auto_accept(state.auto_accept);
    window.set_autostart(state.autostart);
    window.set_update_line(state.update_line.as_deref().unwrap_or_default().into());
    window.set_ltk_line(state.ltk_line.as_deref().unwrap_or_default().into());
    window.set_ltk_download(state.ltk_download.as_deref().unwrap_or_default().into());
    let checks: Vec<PanelCheckRow> = state
        .checks
        .iter()
        .map(|check| PanelCheckRow {
            label: check.label.as_str().into(),
            ok: check.ok,
            detail: check.detail.as_str().into(),
        })
        .collect();
    window.set_checks(ModelRc::new(VecModel::from(checks)));
}

pub fn show_panel(links: PanelLinks) {
    if let Err(e) = runtime::run_on_ui(move || {
        let raised = OPEN_PANEL.with_borrow(|open| open.as_ref().map(|panel| raise(&panel.window)));
        if raised.is_none() {
            open(links);
        }
    }) {
        warn!(error = %e, "Control panel could not be opened");
    }
}

fn raise(window: &ControlPanel) {
    window
        .window()
        .with_winit_window(|w| w.set_minimized(false));
    if let Some(hwnd) = runtime::hwnd_now(window.window()) {
        crate::client_window::take_foreground(hwnd);
    }
}

fn open(links: PanelLinks) {
    let window = match ControlPanel::new() {
        Ok(window) => window,
        Err(e) => {
            warn!(error = %e, "Control panel could not be created");
            return;
        }
    };
    runtime::repaint_on_expose(&window, |w| w.set_expose_flip(!w.get_expose_flip()));

    let last = std::rc::Rc::new(RefCell::new(None::<PanelSnapshot>));
    let language = std::rc::Rc::new(std::cell::Cell::new(None::<crate::i18n::Language>));
    let refresh = {
        let weak = window.as_weak();
        let snapshot = links.snapshot.clone();
        let last = last.clone();
        move || {
            let Some(window) = weak.upgrade() else { return };
            follow_language(&window, &language, crate::i18n::active_language());
            let state = snapshot();
            if last.borrow().as_ref() != Some(&state) {
                render(&window, &state);
                *last.borrow_mut() = Some(state);
            }
        }
    };
    refresh();

    let events = links.events.clone();
    let after_action = refresh.clone();
    window.on_action(move |action| {
        let event = event_for(action);
        if events.send(event).is_err() {
            warn!(
                ?event,
                "Control panel: Dekan is shutting down; the action was not taken"
            );
        }
        after_action();
    });

    window.window().on_close_requested(|| {
        OPEN_PANEL.with_borrow_mut(|open| {
            if let Some(panel) = open.take() {
                slint::Timer::single_shot(Duration::ZERO, move || drop(panel));
            }
        });
        slint::CloseRequestResponse::HideWindow
    });

    if let Err(e) = window.show() {
        warn!(error = %e, "Control panel could not be shown");
        return;
    }
    runtime::when_created(&window, |_, hwnd| {
        crate::client_window::take_foreground(hwnd)
    });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, REFRESH, refresh);
    OPEN_PANEL.with_borrow_mut(|open| {
        *open = Some(OpenPanel {
            window,
            _refresh: timer,
        });
    });
    debug!("Control panel opened");
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
