use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use dekan_core::party::PartyStatus;
use dekan_core::state::{StateReceiver, StateSender, set_party_status};
use dekan_party::client::{PartyClient, PartyExit};
use dekan_party::token::{PartyToken, random_member_id, unix_now};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyCommand {
    Create,
    Join,
    Leave,
}

struct Running {
    stop: CancellationToken,
    task: Pin<Box<dyn Future<Output = PartyExit> + Send>>,
}

async fn next_finished(running: &mut Option<Running>) -> PartyExit {
    match running {
        Some(r) => r.task.as_mut().await,
        None => std::future::pending().await,
    }
}

pub struct PartyManager {
    state_tx: StateSender,
    state_rx: StateReceiver,
    state_dir: PathBuf,
}

impl PartyManager {
    #[must_use]
    pub fn new(state_tx: StateSender, state_rx: StateReceiver, state_dir: PathBuf) -> Self {
        Self {
            state_tx,
            state_rx,
            state_dir,
        }
    }

    pub async fn run(
        self,
        cancel: CancellationToken,
        mut commands: UnboundedReceiver<PartyCommand>,
    ) {
        let mut running: Option<Running> = None;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                command = commands.recv() => {
                    let Some(command) = command else { break };
                    info!(?command, "Party command from the tray");
                    match command {
                        PartyCommand::Create => self.create(&mut running).await,
                        PartyCommand::Join => self.join(&mut running).await,
                        PartyCommand::Leave => Self::stop(&mut running).await,
                    }
                }
                exit = next_finished(&mut running) => {
                    running = None;
                    if exit == PartyExit::RoomFull {

                        let ui = dekan_platform::i18n::text();
                        notify(ui.party_join_title, ui.party_room_full.to_owned());
                    }
                }
            }
        }
        Self::stop(&mut running).await;
    }

    async fn stop(running: &mut Option<Running>) {
        if let Some(current) = running.take() {
            current.stop.cancel();

            let exit = current.task.await;
            debug!(?exit, "Party connection stopped");
        }
    }

    fn relay_or_explain(&self) -> Option<String> {
        match dekan_party::config::relay_url(&self.state_dir) {
            Ok(url) => Some(url),
            Err(e) => {
                warn!(error = %e, "Party mode unavailable: no relay configured");
                set_party_status(
                    &self.state_tx,
                    PartyStatus::Unavailable {
                        reason: e.to_string(),
                    },
                );
                let text = dekan_platform::i18n::text();
                notify(
                    text.party_unavailable_title,
                    dekan_platform::i18n::fill(
                        text.party_unavailable_body,
                        "reason",
                        &e.to_string(),
                    ),
                );
                None
            }
        }
    }

    /// The OS random generator failed: nothing secret can be made, so the party is not started.
    fn explain_no_randomness(&self, e: &dekan_party::error::PartyError) {
        error!(error = %e, "Party not started: no secure randomness for the room key or member id");
        set_party_status(
            &self.state_tx,
            PartyStatus::Unavailable {
                reason: e.to_string(),
            },
        );
        let text = dekan_platform::i18n::text();
        notify(
            text.party_unavailable_title,
            dekan_platform::i18n::fill(text.party_unavailable_body, "reason", &e.to_string()),
        );
    }

    fn start(&self, running: &mut Option<Running>, relay: String, token: PartyToken) -> bool {
        let member_id = match random_member_id() {
            Ok(id) => id,
            Err(e) => {
                self.explain_no_randomness(&e);
                return false;
            }
        };
        let stop = CancellationToken::new();
        let client = PartyClient::new(
            relay,
            token,
            member_id,
            self.state_tx.clone(),
            self.state_rx.clone(),
        );
        *running = Some(Running {
            task: Box::pin(client.run(stop.clone())),
            stop,
        });
        true
    }

    async fn create(&self, running: &mut Option<Running>) {
        let Some(relay) = self.relay_or_explain() else {
            return;
        };
        Self::stop(running).await;

        let token = match random_member_id().and_then(|host| PartyToken::generate(host, unix_now()))
        {
            Ok(token) => token,
            Err(e) => {
                self.explain_no_randomness(&e);
                return;
            }
        };
        let code = token.encode();

        let copied = tokio::task::spawn_blocking({
            let code = code.clone();
            move || dekan_platform::clipboard::set_text(&code)
        })
        .await;
        match copied {
            Ok(Ok(())) => info!("Party room created; code copied to the clipboard"),
            Ok(Err(e)) => warn!(error = %e, "Party room created; the code could not be copied"),
            Err(e) => warn!(error = %e, "Party room created; the clipboard copy task failed"),
        }

        if self.start(running, relay, token) {
            show_created_dialog(code);
        }
    }

    async fn join(&self, running: &mut Option<Running>) {
        let Some(relay) = self.relay_or_explain() else {
            return;
        };

        let clipboard_text =
            match tokio::task::spawn_blocking(dekan_platform::clipboard::get_text).await {
                Ok(Ok(text)) => text,
                Ok(Err(e)) => {
                    debug!(error = %e, "Party join: clipboard not readable; no pre-fill");
                    None
                }
                Err(e) => {
                    debug!(error = %e, "Party join: clipboard read task failed; no pre-fill");
                    None
                }
            };
        let prefill = clipboard_text
            .as_deref()
            .map(str::trim)
            .filter(|c| c.starts_with("DEKAN1:"))
            .map(str::to_string);

        let dialog_prefill = prefill.clone();
        let entered = tokio::task::spawn_blocking(move || {
            dekan_platform::party_dialog::show_party_join_dialog(dialog_prefill.as_deref())
        })
        .await;

        let ui = dekan_platform::i18n::text();
        let outcome: Result<String, String> = match entered {
            Ok(Ok(Some(code))) => Ok(code),
            Ok(Ok(None)) => {
                debug!("Party join cancelled in the dialog");
                return;
            }
            Ok(Err(e)) => Err(e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let code = match outcome {
            Ok(code) => code,

            Err(error) => {
                warn!(error = %error, "Party join dialog failed");
                match prefill {
                    Some(code) => {
                        info!("Party join: using the room code on the clipboard instead");
                        code
                    }
                    None => {
                        notify(
                            ui.party_join_title,
                            dekan_platform::i18n::fill(
                                ui.party_join_clipboard_error,
                                "error",
                                &error,
                            ),
                        );
                        return;
                    }
                }
            }
        };

        match PartyToken::decode(&code, unix_now()) {
            Ok(token) => {
                Self::stop(running).await;
                info!(
                    issued_at = token.issued_at,
                    "Joining a party room from an entered code"
                );
                if self.start(running, relay, token) {
                    notify(ui.party_join_title, ui.party_joining.to_owned());
                }
            }
            Err(e) => {
                warn!(error = %e, "Entered party code refused");
                notify(
                    ui.party_join_title,
                    dekan_platform::i18n::fill(ui.party_invalid_code, "error", &e.to_string()),
                );
            }
        }
    }
}

fn show_created_dialog(code: String) {
    drop(tokio::task::spawn_blocking(move || {
        if let Err(e) = dekan_platform::party_dialog::show_party_created_dialog(&code) {
            warn!(error = %e, "Party room dialog could not be shown; the code goes in a notice");
            let ui = dekan_platform::i18n::text();

            dekan_platform::shell::message_box(
                ui.party_created_title,
                &dekan_platform::i18n::fill(ui.party_copy_failed_body, "code", &code),
            );
        }
    }));
}

fn notify(title: &'static str, text: String) {
    drop(tokio::task::spawn_blocking(move || {
        dekan_platform::shell::message_box(title, &text);
    }));
}
