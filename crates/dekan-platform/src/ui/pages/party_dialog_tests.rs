use super::*;
use crate::i18n::Language;
use i_slint_backend_testing::ElementHandle;

fn open(content: &PartyContent) -> (PartyDialog, mpsc::Receiver<Option<String>>) {
    i_slint_backend_testing::init_no_event_loop();
    let dialog = PartyDialog::new().expect("party dialog");
    let (tx, rx) = mpsc::channel();
    wire(&dialog, content, Answer::new(tx));
    (dialog, rx)
}

fn press(dialog: &PartyDialog, label: &str) {
    ElementHandle::find_by_accessible_label(dialog, label)
        .next()
        .unwrap_or_else(|| panic!("no button labelled {label:?}"))
        .invoke_accessible_default_action();
}

#[test]
fn joining_refuses_an_empty_code_then_answers_the_trimmed_one() {
    let text = Language::English.text();
    let (dialog, answers) = open(&join_content(text, None));
    assert!(!dialog.get_read_only());

    press(&dialog, text.party_dialog_btn_join);
    assert!(dialog.get_show_error(), "an empty code is refused in place");
    assert!(
        answers.try_recv().is_err(),
        "nothing is answered for an empty code"
    );

    dialog.set_code("  BLT-7Q2M-X9KD \n".into());
    press(&dialog, text.party_dialog_btn_join);
    assert_eq!(
        answers.try_recv().ok(),
        Some(Some("BLT-7Q2M-X9KD".to_owned()))
    );
}

#[test]
fn cancelling_answers_nothing_and_only_the_first_answer_counts() {
    let text = Language::English.text();
    let (dialog, answers) = open(&join_content(text, Some("BLT-AAAA")));
    assert_eq!(dialog.get_code(), "BLT-AAAA");
    press(&dialog, text.party_dialog_btn_cancel);
    press(&dialog, text.party_dialog_btn_join);
    assert_eq!(answers.try_recv().ok(), Some(None));
    assert!(answers.try_recv().is_err(), "a second answer is dropped");
}

#[test]
fn the_created_dialog_is_read_only_without_cancel_and_confirms_the_code() {
    let text = Language::Turkish.text();
    let content = created_content(text, "BLT-7Q2M-X9KD");
    assert_eq!(content.inline, InlineAction::Copy);
    let (dialog, answers) = open(&content);
    assert!(dialog.get_read_only());
    assert!(
        ElementHandle::find_by_accessible_label(&dialog, text.party_dialog_btn_cancel)
            .next()
            .is_none(),
        "the created dialog has no cancel button"
    );
    press(&dialog, text.party_dialog_btn_ok);
    assert_eq!(
        answers.try_recv().ok(),
        Some(Some("BLT-7Q2M-X9KD".to_owned()))
    );
}

#[test]
fn every_language_fills_both_dialogs() {
    for language in [Language::Turkish, Language::English] {
        let text = language.text();
        for content in [created_content(text, "X"), join_content(text, None)] {
            for value in [
                content.heading,
                content.label,
                content.inline_label,
                content.confirm,
                content.copied,
                content.empty_error,
            ] {
                assert!(!value.is_empty(), "{language:?} {content:?}");
            }
        }
    }
}

fn key(dialog: &PartyDialog, text: &str) {
    use slint::platform::WindowEvent;
    let window = dialog.window();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}

#[test]
fn enter_confirms_the_typed_code() {
    let text = Language::English.text();
    let (dialog, answers) = open(&join_content(text, None));
    dialog.invoke_focus_field();
    for c in ["B", "L", "T", "-", "9"] {
        key(&dialog, c);
    }
    assert_eq!(
        dialog.get_code(),
        "BLT-9",
        "typed keys reach the code field"
    );
    key(&dialog, "\n");
    assert_eq!(answers.try_recv().ok(), Some(Some("BLT-9".to_owned())));
}

#[test]
fn escape_cancels_from_the_code_field() {
    let text = Language::English.text();
    let (dialog, answers) = open(&join_content(text, None));
    dialog.invoke_focus_field();
    key(&dialog, "\u{1b}");
    assert_eq!(answers.try_recv().ok(), Some(None));
}

#[test]
fn closing_the_window_answers_nothing() {
    let text = Language::English.text();
    let (dialog, answers) = open(&join_content(text, Some("BLT-1")));
    dialog
        .window()
        .dispatch_event(slint::platform::WindowEvent::CloseRequested);
    assert_eq!(answers.try_recv().ok(), Some(None));
}

#[test]
fn a_long_code_never_runs_under_the_inline_button() {
    use i_slint_backend_testing::{AccessibleRole, ElementRoot};
    let text = Language::Turkish.text();
    let code = "DEKAN1:AQAAAABqxLgJABIZ_75__ZlyDx7-Ja4WqQ26WNsR4D8nQkLmN0pQrStUvWxYz";
    let (dialog, _answers) = open(&created_content(text, code));
    let field = dialog
        .root_element()
        .query_descendants()
        .match_accessible_role(AccessibleRole::TextInput)
        .find_first()
        .expect("the code field");
    let button = ElementHandle::find_by_accessible_label(&dialog, text.party_dialog_btn_copy)
        .next()
        .expect("the copy button");
    let field_right = field.absolute_position().x + field.size().width;
    assert!(
        field_right <= button.absolute_position().x,
        "the text area ends at {field_right} but the button starts at {}",
        button.absolute_position().x
    );
}
