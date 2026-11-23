use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, VecModel};
use tracing::{debug, warn};

use super::runtime;
use super::views::{
    ControlPanel, PanelAction, PanelCheckRow, PanelLabels, PanelSkinStatRow, Theme,
};
use crate::tray::TrayEvent;

const REFRESH: Duration = Duration::from_millis(700);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelCheck {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSkinStat {
    pub name: String,
    pub wins: u32,
    pub losses: u32,
    pub winrate: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanelStats {
    pub wins: u32,
    pub losses: u32,
    pub winrate: u8,
    pub top: Vec<PanelSkinStat>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanelRepair {
    pub skins_busy: bool,
    pub client_busy: bool,
    pub state: u8,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSnapshot {
    pub status: String,
    pub party_line: String,
    pub in_room: bool,
    pub auto_accept: bool,
    pub injection: bool,
    pub random_skin: bool,
    pub light_loading: bool,
    pub in_client_ui: bool,
    pub autostart: bool,
    pub update_line: Option<String>,
    pub checks: Vec<PanelCheck>,
    pub stats: PanelStats,
    pub repair: PanelRepair,
}

pub type SnapshotSource = Arc<dyn Fn() -> PanelSnapshot + Send + Sync>;

#[derive(Clone)]
pub struct PanelLinks {
    pub events: tokio::sync::mpsc::UnboundedSender<TrayEvent>,
    pub snapshot: SnapshotSource,
}

struct OpenPanel {
    window: ControlPanel,
    refresh_now: std::rc::Rc<dyn Fn()>,
    _refresh: slint::Timer,
}

thread_local! {
    static OPEN_PANEL: RefCell<Option<OpenPanel>> = const { RefCell::new(None) };
}

#[must_use]
pub(crate) fn event_for(action: PanelAction) -> TrayEvent {
    match action {
        PanelAction::ToggleInjection => TrayEvent::ToggleInjection,
        PanelAction::ToggleRandomSkin => TrayEvent::ToggleRandomSkin,
        PanelAction::ToggleLightLoading => TrayEvent::ToggleLightLoading,
        PanelAction::ToggleInClientUi => TrayEvent::ToggleInClientUi,
        PanelAction::ToggleAutoAccept => TrayEvent::ToggleAutoAccept,
        PanelAction::ToggleAutostart => TrayEvent::ToggleAutostart,
        PanelAction::PartyCreate => TrayEvent::PartyCreate,
        PanelAction::PartyJoin => TrayEvent::PartyJoin,
        PanelAction::PartyLeave => TrayEvent::PartyLeave,
        PanelAction::OpenMods => TrayEvent::OpenMods,
        PanelAction::RestoreMods => TrayEvent::RestoreMods,
        PanelAction::OpenTools => TrayEvent::OpenTools,
        PanelAction::OpenLogs => TrayEvent::OpenLogs,
        PanelAction::OpenRelease => TrayEvent::OpenRelease,
        PanelAction::MarkProblem => TrayEvent::MarkProblem,
        PanelAction::ExportDiagnostics => TrayEvent::ExportDiagnostics,
        PanelAction::RepairSkins => TrayEvent::RepairSkins,
        PanelAction::RepairClient => TrayEvent::RepairClient,
        PanelAction::OpenProject => TrayEvent::OpenProject,
        PanelAction::About => TrayEvent::About,
        PanelAction::Quit => TrayEvent::Quit,
    }
}

pub(crate) fn labels(text: &crate::i18n::Text) -> PanelLabels {
    PanelLabels {
        version: crate::version::display_version().into(),
        tab_home: text.panel_tab_home.into(),
        tab_options: text.panel_section_options.into(),
        tab_party: text.panel_tab_party.into(),
        tab_help: text.panel_tab_help.into(),
        theme: text.panel_theme.into(),
        theme_classic: text.panel_theme_classic.into(),
        theme_crimson: text.panel_theme_crimson.into(),
        theme_obsidian: text.panel_theme_obsidian.into(),
        section_folders: text.menu_group_folders.into(),
        section_diagnostics: text.panel_section_diagnostics.into(),
        all_clear: text.panel_all_clear.into(),
        attention: text.panel_attention.into(),
        see_diagnostics: text.panel_see_diagnostics.into(),
        party_hint: text.panel_party_hint.into(),
        injection: text.menu_injection.into(),
        injection_hint: text.panel_injection_hint.into(),
        random_skin: text.menu_random_skin.into(),
        random_skin_hint: text.panel_random_skin_hint.into(),
        mode_title: text.panel_mode_title.into(),
        mode_window: text.panel_mode_window.into(),
        in_client_ui: text.menu_in_client_ui.into(),
        in_client_ui_hint: text.panel_in_client_ui_hint.into(),
        light_loading: text.menu_light_loading.into(),
        light_loading_hint: text.panel_light_loading_hint.into(),
        auto_accept: text.menu_auto_accept.into(),
        autostart: text.menu_autostart.into(),
        party_create: text.menu_party_create.into(),
        party_join: text.menu_party_join.into(),
        party_leave: text.menu_party_leave.into(),
        open_mods: text.folder_mods.into(),
        restore_mods: text.folder_restore_mods.into(),
        open_tools: text.folder_tools.into(),
        open_logs: text.folder_logs.into(),
        about: text.menu_about.into(),
        open_project: text.panel_open_project.into(),
        project_star_hint: text.panel_project_star_hint.into(),
        quit: text.menu_quit.into(),
        update_download: text.panel_update_download.into(),
        mark_problem: text.panel_mark_problem.into(),
        mark_problem_hint: text.panel_mark_problem_hint.into(),
        export_diagnostics: text.panel_export_diagnostics.into(),
        repair_skins: text.panel_repair_skins.into(),
        repair_client: text.panel_repair_client.into(),
        repair_running: text.panel_repair_running.into(),
        stats_title: text.panel_stats_title.into(),
        stats_empty: text.panel_stats_empty.into(),
        stats_games: text.panel_stats_games.into(),
        stats_wins_short: text.panel_stats_wins_short.into(),
        stats_losses_short: text.panel_stats_losses_short.into(),
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
    window.set_injection(state.injection);
    window.set_random_skin(state.random_skin);
    window.set_light_loading(state.light_loading);
    window.set_in_client_ui(state.in_client_ui);
    window.set_auto_accept(state.auto_accept);
    window.set_autostart(state.autostart);
    window.set_update_line(state.update_line.as_deref().unwrap_or_default().into());
    let failing = state.checks.iter().filter(|check| !check.ok).count();
    window.set_attention(i32::try_from(failing).unwrap_or(i32::MAX));
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
    window.set_stats_wins(i32::try_from(state.stats.wins).unwrap_or(i32::MAX));
    window.set_stats_losses(i32::try_from(state.stats.losses).unwrap_or(i32::MAX));
    window.set_stats_winrate(i32::from(state.stats.winrate));
    let t = crate::i18n::text();
    let rows: Vec<PanelSkinStatRow> = state
        .stats
        .top
        .iter()
        .map(|s| PanelSkinStatRow {
            name: s.name.as_str().into(),
            record: format!(
                "{}{} / {}{}",
                s.wins, t.panel_stats_wins_short, s.losses, t.panel_stats_losses_short
            )
            .into(),
            winrate: i32::from(s.winrate),
        })
        .collect();
    window.set_stats_rows(ModelRc::new(VecModel::from(rows)));
    window.set_repair_skins_busy(state.repair.skins_busy);
    window.set_repair_client_busy(state.repair.client_busy);
    window.set_repair_state(i32::from(state.repair.state));
    window.set_repair_message(state.repair.message.as_str().into());
}

pub fn refresh_open_panel() {
    // ignore-ok: panel may not be running in headless or shutting down state
    let _ = runtime::run_on_ui(|| {
        let refresh = OPEN_PANEL.with_borrow(|open| open.as_ref().map(|p| p.refresh_now.clone()));
        if let Some(refresh) = refresh {
            refresh();
        }
    });
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
    super::themes::follow(&window, |window, mode| {
        window.global::<Theme>().set_mode(mode)
    });

    window.on_theme_changed(move |mode| {
        let mode = match crate::preferences::set_theme_mode(mode) {
            Ok(mode) => mode,
            Err(e) => {
                warn!(error = %e, "Could not save the selected theme");
                crate::preferences::theme_mode()
            }
        };
        super::themes::apply(mode);
    });

    runtime::repaint_on_expose(&window, |w| w.set_expose_flip(!w.get_expose_flip()));

    let last = std::rc::Rc::new(RefCell::new(None::<PanelSnapshot>));
    let language = std::rc::Rc::new(std::cell::Cell::new(None::<crate::i18n::Language>));
    let refresh: std::rc::Rc<dyn Fn()> = std::rc::Rc::new({
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
    });
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
    let timer_refresh = refresh.clone();
    timer.start(slint::TimerMode::Repeated, REFRESH, move || timer_refresh());
    OPEN_PANEL.with_borrow_mut(|open| {
        *open = Some(OpenPanel {
            window,
            refresh_now: refresh,
            _refresh: timer,
        });
    });
    debug!("Control panel opened");
}

#[cfg(test)]
mod repair_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod theme_tests;

#[cfg(test)]
mod mode_tests;
