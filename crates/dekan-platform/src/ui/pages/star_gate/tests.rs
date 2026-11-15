use super::*;
use crate::i18n::Language;
use i_slint_backend_testing::ElementHandle;

fn press(window: &StarGateWindow, label: &str) {
    ElementHandle::find_by_accessible_label(window, label)
        .next()
        .unwrap_or_else(|| panic!("button {label}"))
        .invoke_accessible_default_action();
}

#[test]
fn every_dictionary_fills_the_window_without_empty_labels() {
    i_slint_backend_testing::init_no_event_loop();
    for language in [Language::Portuguese, Language::Spanish, Language::English] {
        let window = StarGateWindow::new().expect("star gate window");
        fill(&window, language.text());
        for label in [
            window.get_heading(),
            window.get_intro(),
            window.get_sign_in_label(),
            window.get_open_repo_label(),
            window.get_recheck_label(),
            window.get_quit_label(),
            window.get_code_hint(),
        ] {
            assert!(!label.is_empty(), "{language:?}");
        }
    }
}

#[test]
fn buttons_report_their_events_and_recheck_waits_for_a_token() {
    i_slint_backend_testing::init_no_event_loop();
    let text = Language::English.text();
    let window = StarGateWindow::new().expect("star gate window");
    fill(&window, text);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    wire(&window, &tx);

    apply(&window, &GateView::default());
    press(&window, text.star_gate_recheck);
    assert!(
        rx.try_recv().is_err(),
        "recheck is disabled without a token"
    );

    press(&window, text.star_gate_sign_in);
    assert_eq!(rx.try_recv().ok(), Some(GateEvent::SignIn));

    apply(
        &window,
        &GateView {
            can_recheck: true,
            ..GateView::default()
        },
    );
    press(&window, text.star_gate_recheck);
    assert_eq!(rx.try_recv().ok(), Some(GateEvent::Recheck));

    press(&window, text.star_gate_quit);
    assert_eq!(rx.try_recv().ok(), Some(GateEvent::Quit));
}

#[test]
fn a_busy_view_shows_the_code_and_blocks_another_sign_in() {
    i_slint_backend_testing::init_no_event_loop();
    let text = Language::English.text();
    let window = StarGateWindow::new().expect("star gate window");
    fill(&window, text);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    wire(&window, &tx);
    apply(
        &window,
        &GateView {
            code: "ABCD-1234".into(),
            message: text.star_gate_waiting.into(),
            busy: true,
            ..GateView::default()
        },
    );
    assert_eq!(window.get_code(), "ABCD-1234");
    press(&window, text.star_gate_sign_in);
    assert!(rx.try_recv().is_err());
}
