use super::*;
use crate::i18n::Language;
use i_slint_backend_testing::ElementHandle;
use slint::Model;

const ACTIONS: [PanelAction; 16] = [
    PanelAction::ToggleRandomSkin,
    PanelAction::ToggleLightLoading,
    PanelAction::ToggleAutoAccept,
    PanelAction::ToggleAutostart,
    PanelAction::PartyCreate,
    PanelAction::PartyJoin,
    PanelAction::PartyLeave,
    PanelAction::OpenMods,
    PanelAction::OpenTools,
    PanelAction::OpenLogs,
    PanelAction::OpenRelease,
    PanelAction::InstallInjector,
    PanelAction::MarkProblem,
    PanelAction::ExportDiagnostics,
    PanelAction::About,
    PanelAction::Quit,
];

fn tall_panel() -> ControlPanel {
    i_slint_backend_testing::init_no_event_loop();
    let window = ControlPanel::new().expect("panel");
    window
        .window()
        .set_size(slint::LogicalSize::new(460.0, 1600.0));
    window
}

fn snapshot() -> PanelSnapshot {
    PanelSnapshot {
        status: "Champion select".into(),
        party_line: "Room BLT-7Q2M".into(),
        in_room: true,
        auto_accept: true,
        random_skin: false,
        light_loading: true,
        autostart: false,
        update_line: Some("Dekan 1.3 is available".into()),
        ltk_line: None,
        ltk_download: None,
        checks: vec![
            PanelCheck {
                label: "Injector".into(),
                ok: true,
                detail: "audited".into(),
            },
            PanelCheck {
                label: "DLL".into(),
                ok: false,
                detail: "refused".into(),
            },
        ],
    }
}

#[test]
fn every_panel_action_reaches_a_distinct_tray_event() {
    let events: Vec<String> = ACTIONS
        .iter()
        .map(|action| format!("{:?}", event_for(*action)))
        .collect();
    let mut unique = events.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ACTIONS.len(), "{events:?}");
}

#[test]
fn every_language_fills_every_label() {
    for language in [Language::Turkish, Language::English] {
        let labels = labels(language.text());
        for (name, value) in [
            ("options", &labels.section_options),
            ("party", &labels.section_party),
            ("folders", &labels.section_folders),
            ("diagnostics", &labels.section_diagnostics),
            ("random", &labels.random_skin),
            ("light", &labels.light_loading),
            ("accept", &labels.auto_accept),
            ("autostart", &labels.autostart),
            ("create", &labels.party_create),
            ("join", &labels.party_join),
            ("leave", &labels.party_leave),
            ("about", &labels.about),
            ("quit", &labels.quit),
            ("mark", &labels.mark_problem),
            ("export", &labels.export_diagnostics),
        ] {
            assert!(!value.is_empty(), "{language:?} {name}");
        }
    }
}

#[test]
fn the_real_panel_shows_the_snapshot_and_a_toggle_sends_its_action() {
    let window = tall_panel();
    let text = Language::English.text();
    window.set_labels(labels(text));
    render(&window, &snapshot());

    assert_eq!(window.get_status(), "Champion select");
    assert_eq!(window.get_checks().row_count(), 2);
    let light = ElementHandle::find_by_accessible_label(&window, text.menu_light_loading)
        .next()
        .expect("light loading toggle");
    assert_eq!(light.accessible_checked(), Some(true));
    let random = ElementHandle::find_by_accessible_label(&window, text.menu_random_skin)
        .next()
        .expect("random skin toggle");
    assert_eq!(random.accessible_checked(), Some(false));

    let fired = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = fired.clone();
    window.on_action(move |action| sink.borrow_mut().push(action));
    random.invoke_accessible_default_action();
    ElementHandle::find_by_accessible_label(&window, text.menu_quit)
        .next()
        .expect("quit button")
        .invoke_accessible_default_action();
    ElementHandle::find_by_accessible_label(&window, text.panel_update_download)
        .next()
        .expect("the update notice offers its download")
        .invoke_accessible_default_action();
    assert_eq!(
        *fired.borrow(),
        vec![
            PanelAction::ToggleRandomSkin,
            PanelAction::Quit,
            PanelAction::OpenRelease
        ]
    );
}

#[test]
fn party_buttons_follow_the_room_state() {
    let window = tall_panel();
    let text = Language::English.text();
    window.set_labels(labels(text));
    let fired = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = fired.clone();
    window.on_action(move |action| sink.borrow_mut().push(action));

    let press = |label: &str| {
        ElementHandle::find_by_accessible_label(&window, label)
            .next()
            .expect("party button")
            .invoke_accessible_default_action();
    };
    render(&window, &snapshot());
    press(text.menu_party_create);
    press(text.menu_party_leave);
    render(
        &window,
        &PanelSnapshot {
            in_room: false,
            ..snapshot()
        },
    );
    press(text.menu_party_create);
    press(text.menu_party_leave);
    assert_eq!(
        *fired.borrow(),
        vec![PanelAction::PartyLeave, PanelAction::PartyCreate]
    );
}

#[test]
fn every_button_and_toggle_of_the_panel_sends_its_own_action() {
    let window = tall_panel();
    let text = Language::English.text();
    window.set_labels(labels(text));
    let state = PanelSnapshot {
        in_room: false,
        ltk_line: Some("LTK Manager 1.27 has a new injector.".into()),
        ltk_download: Some("Install injector 1.27".into()),
        ..snapshot()
    };
    render(&window, &state);
    let fired = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = fired.clone();
    window.on_action(move |action| sink.borrow_mut().push(action));

    let clicks = [
        (text.panel_update_download, PanelAction::OpenRelease),
        ("Install injector 1.27", PanelAction::InstallInjector),
        (text.menu_random_skin, PanelAction::ToggleRandomSkin),
        (text.menu_light_loading, PanelAction::ToggleLightLoading),
        (text.menu_auto_accept, PanelAction::ToggleAutoAccept),
        (text.menu_autostart, PanelAction::ToggleAutostart),
        (text.menu_party_create, PanelAction::PartyCreate),
        (text.menu_party_join, PanelAction::PartyJoin),
        (text.folder_mods, PanelAction::OpenMods),
        (text.folder_tools, PanelAction::OpenTools),
        (text.folder_logs, PanelAction::OpenLogs),
        (text.panel_mark_problem, PanelAction::MarkProblem),
        (
            text.panel_export_diagnostics,
            PanelAction::ExportDiagnostics,
        ),
        (text.menu_about, PanelAction::About),
        (text.menu_quit, PanelAction::Quit),
    ];
    for (label, _) in clicks {
        ElementHandle::find_by_accessible_label(&window, label)
            .next()
            .unwrap_or_else(|| panic!("no control labelled {label:?}"))
            .invoke_accessible_default_action();
    }
    let expected: Vec<PanelAction> = clicks.iter().map(|(_, action)| *action).collect();
    assert_eq!(*fired.borrow(), expected);

    render(
        &window,
        &PanelSnapshot {
            in_room: true,
            ..state
        },
    );
    ElementHandle::find_by_accessible_label(&window, text.menu_party_leave)
        .next()
        .expect("leave button")
        .invoke_accessible_default_action();
    assert_eq!(fired.borrow().last(), Some(&PanelAction::PartyLeave));
}

#[test]
fn notices_appear_only_with_a_line_and_the_ltk_button_only_with_a_download() {
    let window = tall_panel();
    let text = Language::English.text();
    window.set_labels(labels(text));
    let quiet = PanelSnapshot {
        update_line: None,
        ..snapshot()
    };
    render(&window, &quiet);
    assert!(
        ElementHandle::find_by_accessible_label(&window, text.panel_update_download)
            .next()
            .is_none()
    );
    render(
        &window,
        &PanelSnapshot {
            ltk_line: Some("Injector missing".into()),
            ..quiet
        },
    );
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Injector missing")
            .next()
            .is_some()
    );
    assert_eq!(window.get_ltk_download(), "");
}

#[test]
fn long_diagnostic_lines_never_run_under_the_hint_below_them() {
    let window = tall_panel();
    let text = Language::English.text();
    window.set_labels(labels(text));
    let details = [
        "OK",
        "OK",
        "waiting for the client to open",
        "accepts the current patch; refuses game builds made 0 day(s) from now or later",
        "LTK Manager 1.27.0 ships a newer injector signed by League Toolkit",
        "running as administrator",
    ];
    let state = PanelSnapshot {
        checks: details
            .iter()
            .map(|detail| PanelCheck {
                label: "Injector DLL validity".into(),
                ok: false,
                detail: (*detail).into(),
            })
            .collect(),
        ..snapshot()
    };
    render(&window, &state);
    let bottom = |label: &str| {
        let element = ElementHandle::find_by_accessible_label(&window, label)
            .next()
            .unwrap_or_else(|| panic!("{label} shown"));
        element.absolute_position().y + element.size().height
    };
    let top = |label: &str| {
        ElementHandle::find_by_accessible_label(&window, label)
            .next()
            .unwrap_or_else(|| panic!("{label} shown"))
            .absolute_position()
            .y
    };
    let last = bottom(details[5]);
    let hint = top(text.panel_mark_problem_hint);
    assert!(
        hint >= last,
        "hint at {hint} starts above the last check ending at {last}"
    );
    assert!(
        bottom(details[3]) <= top(details[4]) - 1.0,
        "a wrapped detail runs into the next row"
    );
}

#[test]
fn labels_follow_the_client_language_once_it_is_known() {
    let window = tall_panel();
    let shown = std::cell::Cell::new(None);
    follow_language(&window, &shown, Language::Turkish);
    assert_eq!(
        window.get_labels().section_diagnostics,
        Language::Turkish.text().panel_section_diagnostics
    );
    follow_language(&window, &shown, Language::English);
    assert_eq!(
        window.get_labels().section_diagnostics,
        Language::English.text().panel_section_diagnostics
    );
    assert_eq!(shown.get(), Some(Language::English));
}
