use super::tests::tall_panel;
use super::*;
use crate::i18n::Language;
use i_slint_backend_testing::ElementHandle;

#[test]
fn options_keep_dekan_theme_callbacks_in_both_languages() {
    for language in [Language::Turkish, Language::English] {
        let window = tall_panel();
        let text = language.text();
        window.set_labels(labels(text));
        window.set_tab(1);
        let modes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = modes.clone();
        window.on_theme_changed(move |mode| sink.borrow_mut().push(mode));
        for label in [
            text.panel_theme_classic,
            text.panel_theme_crimson,
            text.panel_theme_obsidian,
        ] {
            ElementHandle::find_by_accessible_label(&window, label)
                .next()
                .unwrap_or_else(|| panic!("no theme button labelled {label:?}"))
                .invoke_accessible_default_action();
        }
        assert_eq!(*modes.borrow(), vec![0, 1, 2]);
    }
}
