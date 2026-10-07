use super::*;
use crate::i18n::Language;
use i_slint_backend_testing::ElementHandle;
use slint::Model;

#[test]
fn about_differs_from_the_intro_and_skips_empty_rows() {
    for language in [Language::Turkish, Language::English] {
        let text = language.text();
        let about = about_content(text);
        let welcome = welcome_content(text);
        assert_ne!(about, welcome, "{language:?}");
        assert!(
            about
                .items
                .iter()
                .chain(&welcome.items)
                .all(|item| !item.is_empty())
        );
        assert!(
            !about.items.contains(&text.welcome_tray_hint),
            "{language:?}: no tray hint in About"
        );
        assert!(
            about.items.iter().any(|item| item.contains("Miss Fortune")),
            "{language:?}"
        );
    }
    assert!(
        !welcome_content(Language::English.text())
            .items
            .iter()
            .any(|item| item.contains("Miss Fortune")),
        "no guessed translation of the quote"
    );
}

#[test]
fn the_real_window_shows_every_row_and_dismisses_from_its_button() {
    i_slint_backend_testing::init_no_event_loop();
    let text = Language::English.text();
    let content = about_content(text);
    let window = WelcomeWindow::new().expect("welcome window");
    fill(&window, &content);
    assert_eq!(window.get_items().row_count(), content.items.len());
    assert_eq!(window.get_version(), crate::version::display_version());

    let dismissed = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = dismissed.clone();
    window.on_dismiss(move || count.set(count.get() + 1));
    ElementHandle::find_by_accessible_label(&window, text.about_dismiss)
        .next()
        .expect("dismiss button")
        .invoke_accessible_default_action();
    assert_eq!(dismissed.get(), 1);
}

#[test]
fn tab_reaches_the_dismiss_button_and_space_presses_it() {
    use slint::platform::WindowEvent;
    i_slint_backend_testing::init_no_event_loop();
    let window = WelcomeWindow::new().expect("welcome window");
    fill(&window, &about_content(Language::English.text()));
    let dismissed = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = dismissed.clone();
    window.on_dismiss(move || count.set(count.get() + 1));
    window.show().expect("show");
    let key = |text: &str| {
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    };
    key("\t");
    key(" ");
    assert_eq!(
        dismissed.get(),
        1,
        "the focused button reacts to the space bar"
    );
}
