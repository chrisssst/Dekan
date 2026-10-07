use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::ComponentHandle;
use tracing::{debug, warn};

use super::runtime;
use super::views::PartyDialog;
use crate::error::PlatformError;

const COPIED_FOR: Duration = Duration::from_millis(2200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineAction {
    Copy,
    Paste,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartyContent {
    pub heading: &'static str,
    pub description: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub read_only: bool,
    pub inline: InlineAction,
    pub inline_label: &'static str,
    pub confirm: &'static str,
    pub cancel: &'static str,
    pub copied: &'static str,
    pub empty_error: &'static str,
    pub code: String,
}

pub(crate) fn created_content(text: &crate::i18n::Text, code: &str) -> PartyContent {
    PartyContent {
        heading: text.party_dialog_create_title,
        description: text.party_dialog_create_desc,
        label: text.party_dialog_label_code,
        placeholder: "",
        read_only: true,
        inline: InlineAction::Copy,
        inline_label: text.party_dialog_btn_copy,
        confirm: text.party_dialog_btn_ok,
        cancel: "",
        copied: text.party_dialog_copied,
        empty_error: text.party_dialog_error_empty,
        code: code.to_owned(),
    }
}

pub(crate) fn join_content(text: &crate::i18n::Text, initial_code: Option<&str>) -> PartyContent {
    PartyContent {
        heading: text.party_dialog_join_title,
        description: text.party_dialog_join_desc,
        label: text.party_dialog_label_code,
        placeholder: text.party_dialog_placeholder,
        read_only: false,
        inline: InlineAction::Paste,
        inline_label: text.party_dialog_btn_paste,
        confirm: text.party_dialog_btn_join,
        cancel: text.party_dialog_btn_cancel,
        copied: text.party_dialog_copied,
        empty_error: text.party_dialog_error_empty,
        code: initial_code.unwrap_or_default().to_owned(),
    }
}

pub fn show_party_created_dialog(code: &str) -> Result<(), PlatformError> {
    run_modal(created_content(crate::i18n::text(), code)).map(|_| ())
}

pub fn show_party_join_dialog(initial_code: Option<&str>) -> Result<Option<String>, PlatformError> {
    run_modal(join_content(crate::i18n::text(), initial_code))
}

fn run_modal(content: PartyContent) -> Result<Option<String>, PlatformError> {
    let (answer_tx, answer_rx) = mpsc::channel();
    runtime::run_on_ui(move || {
        let answer = Answer::new(answer_tx);
        if let Err(e) = open(&content, answer.clone()) {
            warn!(error = %e, "The party dialog could not be opened");
            answer.give(None);
        }
    })?;
    answer_rx
        .recv()
        .map_err(|_| PlatformError::Window("the party dialog closed without an answer".into()))
}

#[derive(Clone)]
pub(crate) struct Answer(Rc<Cell<Option<mpsc::Sender<Option<String>>>>>);

impl Answer {
    pub(crate) fn new(sender: mpsc::Sender<Option<String>>) -> Self {
        Self(Rc::new(Cell::new(Some(sender))))
    }

    fn give(&self, value: Option<String>) {
        if let Some(sender) = self.0.take() {
            if sender.send(value).is_err() {
                debug!("Nobody is waiting for the party dialog any more");
            }
        }
    }
}

pub(crate) fn wire(dialog: &PartyDialog, content: &PartyContent, answer: Answer) {
    dialog.set_heading(content.heading.into());
    dialog.set_description(content.description.into());
    dialog.set_label(content.label.into());
    dialog.set_placeholder(content.placeholder.into());
    dialog.set_read_only(content.read_only);
    dialog.set_inline_label(content.inline_label.into());
    dialog.set_confirm_label(content.confirm.into());
    dialog.set_cancel_label(content.cancel.into());
    dialog.set_copied_label(content.copied.into());
    dialog.set_empty_error(content.empty_error.into());
    dialog.set_code(content.code.as_str().into());

    let inline = content.inline;
    let weak = dialog.as_weak();
    dialog.on_inline_action(move || {
        if let Some(dialog) = weak.upgrade() {
            run_inline(&dialog, inline);
        }
    });

    let weak = dialog.as_weak();
    let on_submit = answer.clone();
    dialog.on_submit(move |code| {
        on_submit.give(Some(code.trim().to_owned()));
        if let Some(dialog) = weak.upgrade() {
            close(&dialog);
        }
    });

    let weak = dialog.as_weak();
    let on_cancel = answer.clone();
    dialog.on_cancel(move || {
        on_cancel.give(None);
        if let Some(dialog) = weak.upgrade() {
            close(&dialog);
        }
    });

    dialog.window().on_close_requested(move || {
        answer.give(None);
        slint::CloseRequestResponse::HideWindow
    });
}

fn open(content: &PartyContent, answer: Answer) -> Result<(), slint::PlatformError> {
    let dialog = PartyDialog::new()?;
    runtime::repaint_on_expose(&dialog, |d| d.set_expose_flip(!d.get_expose_flip()));
    wire(&dialog, content, answer);
    dialog.show()?;
    runtime::when_created(&dialog, |dialog, hwnd| {
        crate::client_window::take_foreground(hwnd);
        dialog.invoke_focus_field();
    });
    Ok(())
}

fn run_inline(dialog: &PartyDialog, action: InlineAction) {
    match action {
        InlineAction::Copy => {
            let code = dialog.get_code();
            if code.trim().is_empty() {
                return;
            }
            if let Err(e) = crate::clipboard::set_text(code.trim()) {
                warn!(error = %e, "Party dialog: could not copy the room code to the clipboard");
                return;
            }
            dialog.set_copied(true);
            let weak = dialog.as_weak();
            slint::Timer::single_shot(COPIED_FOR, move || {
                if let Some(dialog) = weak.upgrade() {
                    dialog.set_copied(false);
                }
            });
        }
        InlineAction::Paste => match crate::clipboard::get_text() {
            Ok(Some(text)) => {
                dialog.set_code(text.trim().into());
                dialog.set_show_error(false);
                dialog.invoke_focus_field();
            }
            Ok(None) => debug!("Party dialog: paste requested but the clipboard holds no text"),
            Err(e) => warn!(error = %e, "Party dialog: could not read the clipboard for paste"),
        },
    }
}

fn close(dialog: &PartyDialog) {
    if let Err(e) = dialog.hide() {
        debug!(error = %e, "The party dialog was already closed");
    }
}

#[cfg(test)]
#[path = "party_dialog_tests.rs"]
mod tests;
