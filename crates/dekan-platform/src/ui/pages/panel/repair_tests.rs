use super::*;
use crate::i18n::Language;
use i_slint_backend_testing::ElementHandle;

fn empty_snapshot(repair: PanelRepair) -> PanelSnapshot {
    PanelSnapshot {
        status: "Ready".into(),
        party_line: String::new(),
        in_room: false,
        auto_accept: false,
        injection: true,
        random_skin: false,
        light_loading: false,
        autostart: false,
        update_line: None,
        checks: Vec::new(),
        stats: PanelStats::default(),
        repair,
    }
}

#[test]
fn repair_buttons_show_loading_state_disable_clicks_and_render_result_banner() {
    i_slint_backend_testing::init_no_event_loop();
    let window = ControlPanel::new().expect("panel");
    window
        .window()
        .set_size(slint::LogicalSize::new(460.0, 1600.0));
    let text = Language::Turkish.text();
    window.set_labels(labels(text));
    window.set_tab(3);

    let fired = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = fired.clone();
    window.on_action(move |action| sink.borrow_mut().push(action));

    render(
        &window,
        &empty_snapshot(PanelRepair {
            skins_busy: true,
            client_busy: false,
            state: 1,
            message: text.panel_repair_skins_running.into(),
        }),
    );
    assert!(window.get_repair_skins_busy());
    assert!(!window.get_repair_client_busy());
    assert_eq!(window.get_repair_state(), 1);
    assert_eq!(
        window.get_repair_message().as_str(),
        text.panel_repair_skins_running
    );
    let busy_btn = ElementHandle::find_by_accessible_label(&window, text.panel_repair_running)
        .next()
        .expect("busy repair button");
    busy_btn.invoke_accessible_default_action();
    assert!(
        fired.borrow().is_empty(),
        "disabled repair button must ignore duplicate clicks"
    );

    render(
        &window,
        &empty_snapshot(PanelRepair {
            skins_busy: false,
            client_busy: false,
            state: 2,
            message: "Cliente corrigido".into(),
        }),
    );
    assert!(!window.get_repair_skins_busy());
    assert_eq!(window.get_repair_state(), 2);
    assert_eq!(window.get_repair_message(), "Cliente corrigido");
    ElementHandle::find_by_accessible_label(&window, text.panel_repair_client)
        .next()
        .expect("client repair button re-enabled")
        .invoke_accessible_default_action();
    assert_eq!(*fired.borrow(), vec![PanelAction::RepairClient]);
}
