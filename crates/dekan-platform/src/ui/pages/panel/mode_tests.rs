use super::tests::tall_panel;
use super::*;
use crate::i18n::Language;
use i_slint_backend_testing::ElementHandle;

#[test]
fn mode_card_offers_exclusive_window_and_client_choices_in_both_languages() {
    let window = tall_panel();
    window.set_tab(0);
    for language in [Language::Turkish, Language::English] {
        let text = language.text();
        window.set_labels(labels(text));
        window.set_in_client_ui(false);
        let fired = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = fired.clone();
        window.on_action(move |action| sink.borrow_mut().push(action));
        let standalone = ElementHandle::find_by_accessible_label(&window, text.panel_mode_window)
            .next()
            .expect("Dekan interface choice");
        let client = ElementHandle::find_by_accessible_label(&window, text.menu_in_client_ui)
            .next()
            .expect("native client choice");
        assert_eq!(standalone.accessible_checked(), Some(true));
        assert_eq!(client.accessible_checked(), Some(false));
        standalone.invoke_accessible_default_action();
        assert!(fired.borrow().is_empty());
        client.invoke_accessible_default_action();
        assert_eq!(*fired.borrow(), vec![PanelAction::ToggleInClientUi]);
        window.set_in_client_ui(true);
        assert_eq!(standalone.accessible_checked(), Some(false));
        assert_eq!(client.accessible_checked(), Some(true));
        client.invoke_accessible_default_action();
        assert_eq!(fired.borrow().len(), 1);
        standalone.invoke_accessible_default_action();
        assert_eq!(*fired.borrow(), vec![PanelAction::ToggleInClientUi; 2]);
    }
}
