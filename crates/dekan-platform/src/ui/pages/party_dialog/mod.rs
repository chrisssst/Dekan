use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::ComponentHandle;
use tracing::{debug, warn};

use super::runtime;
use super::views::{PartyDialog, PartyInvite, Theme};
use crate::error::PlatformError;

const COPIED_FOR: Duration = Duration::from_millis(2200);

thread_local! {
    static OPEN_INVITE: RefCell<Option<slint::Weak<PartyInvite>>> = const { RefCell::new(None) };
}

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

pub fn show_lobby_invite() -> Result<bool, PlatformError> {
    let (answer_tx, answer_rx) = mpsc::channel();
    runtime::run_on_ui(move || {
        let answer = Answer::new(answer_tx);
        if let Err(e) = open_invite(crate::i18n::text(), answer.clone()) {
            warn!(error = %e, "The lobby party invite could not be opened");
            answer.give(false);
        }
    })?;
    answer_rx.recv().map_err(|_| {
        PlatformError::Window("the lobby party invite closed without an answer".into())
    })
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

pub fn dismiss_lobby_invite() -> Result<(), PlatformError> {
    runtime::run_on_ui(|| {
        if let Some(invite) = OPEN_INVITE.take().and_then(|weak| weak.upgrade()) {
            invite.invoke_later();
        }
    })
}

#[derive(Clone)]
pub(crate) struct Answer<T>(Rc<Cell<Option<mpsc::Sender<T>>>>);

impl<T> Answer<T> {
    pub(crate) fn new(sender: mpsc::Sender<T>) -> Self {
        Self(Rc::new(Cell::new(Some(sender))))
    }

    fn give(&self, value: T) {
        if let Some(sender) = self.0.take() {
            if sender.send(value).is_err() {
                debug!("Nobody is waiting for the party dialog any more");
            }
        }
    }
}

pub(crate) fn wire(dialog: &PartyDialog, content: &PartyContent, answer: Answer<Option<String>>) {
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

fn open(
    content: &PartyContent,
    answer: Answer<Option<String>>,
) -> Result<(), slint::PlatformError> {
    let dialog = PartyDialog::new()?;
    super::themes::follow(&dialog, |dialog, mode| {
        dialog.global::<Theme>().set_mode(mode)
    });
    runtime::repaint_on_expose(&dialog, |d| d.set_expose_flip(!d.get_expose_flip()));
    wire(&dialog, content, answer);
    dialog.show()?;
    runtime::when_created(&dialog, |dialog, hwnd| {
        crate::client_window::take_foreground(hwnd);
        dialog.invoke_focus_field();
    });
    Ok(())
}

pub(crate) fn wire_invite(invite: &PartyInvite, text: &crate::i18n::Text, answer: Answer<bool>) {
    invite.set_heading(text.party_invite_title.into());
    invite.set_description(text.party_invite_desc.into());
    invite.set_join_label(text.party_invite_join.into());
    invite.set_later_label(text.party_invite_later.into());
    let answer_with = |accepted: bool| {
        let answer = answer.clone();
        let weak = invite.as_weak();
        move || {
            answer.give(accepted);
            if let Some(invite) = weak.upgrade() {
                if let Err(e) = invite.hide() {
                    debug!(error = %e, "The lobby party invite was already closed");
                }
            }
        }
    };
    invite.on_join(answer_with(true));
    invite.on_later(answer_with(false));
    invite.window().on_close_requested(move || {
        answer.give(false);
        slint::CloseRequestResponse::HideWindow
    });
}

fn open_invite(text: &crate::i18n::Text, answer: Answer<bool>) -> Result<(), slint::PlatformError> {
    let invite = PartyInvite::new()?;
    super::themes::follow(&invite, |invite, mode| {
        invite.global::<Theme>().set_mode(mode)
    });
    runtime::repaint_on_expose(&invite, |i| i.set_expose_flip(!i.get_expose_flip()));
    wire_invite(&invite, text, answer);
    invite.show()?;
    OPEN_INVITE.set(Some(invite.as_weak()));
    runtime::when_created(&invite, |_, hwnd| {
        crate::client_window::take_foreground(hwnd)
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
mod tests;
