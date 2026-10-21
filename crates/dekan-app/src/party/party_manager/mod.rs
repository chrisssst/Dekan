use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dekan_core::party::PartyStatus;
use dekan_core::state::{
    StateReceiver, StateSender, set_party_hosting, set_party_share, set_party_status,
};
use dekan_party::client::{PartyClient, PartyExit};
use dekan_party::token::{PartyToken, random_member_id, unix_now};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use lobby::LobbyFollow;
use rooms::DialogOpen;
pub use rooms::{HostedRooms, QueuedClicks, sort_queued_clicks};

mod lobby;
mod rooms;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyCommand {
    Create,
    Join,
    Leave,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Room {
    Code,
    Lobby(String),
}

struct Running {
    stop: CancellationToken,
    task: Pin<Box<dyn Future<Output = PartyExit> + Send>>,
    room: Room,
}

fn in_code_room(running: Option<&Running>) -> bool {
    running.is_some_and(|r| r.room == Room::Code)
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
    created_dialog_open: Arc<AtomicBool>,
    hosted: HostedRooms,
}

impl PartyManager {
    #[must_use]
    pub fn new(state_tx: StateSender, state_rx: StateReceiver, state_dir: PathBuf) -> Self {
        Self {
            state_tx,
            state_rx,
            state_dir,
            created_dialog_open: Arc::new(AtomicBool::new(false)),
            hosted: HostedRooms::default(),
        }
    }

    pub async fn run(
        self,
        cancel: CancellationToken,
        mut commands: UnboundedReceiver<PartyCommand>,
    ) {
        let mut running: Option<Running> = None;
        let mut lobby = LobbyFollow::default();
        let mut state_rx = self.state_rx.clone();
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                command = commands.recv() => {
                    let Some(command) = command else { break };
                    info!(?command, "Party command from the tray");
                    match command {
                        PartyCommand::Create => self.create(&mut running).await,
                        PartyCommand::Join => {
                            self.join(&mut running).await;
                            self.settle_queued_clicks(&mut commands, &mut running, &mut lobby).await;
                        }
                        PartyCommand::Leave => self.leave(&mut running, &mut lobby).await,
                    }
                }
                exit = next_finished(&mut running) => {
                    let room = running.take().map(|r| r.room);
                    set_party_hosting(&self.state_tx, false);
                    set_party_share(&self.state_tx, false);
                    match room {
                        Some(Room::Lobby(party_id)) => {
                            warn!(?exit, "Party: the lobby room closed; it is not reopened for this lobby");
                            lobby.decline(party_id);
                        }
                        _ if exit == PartyExit::RoomFull => {
                            let ui = dekan_platform::i18n::text();
                            notify(ui.party_join_title, ui.party_room_full.to_owned());
                        }
                        _ => {}
                    }
                }
                changed = state_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    self.follow_lobby(&mut running, &mut lobby).await;
                }
                answer = lobby.next_answer() => self.answer_invite(answer, &mut running, &mut lobby).await,
            }
        }
        lobby.dismiss();
        self.stop(&mut running).await;
    }

    async fn leave(&self, running: &mut Option<Running>, lobby: &mut LobbyFollow) {
        if let Some(Room::Lobby(party_id)) = running.as_ref().map(|r| r.room.clone()) {
            lobby.decline(party_id);
        }
        self.stop(running).await;
        set_party_hosting(&self.state_tx, false);
    }

    async fn settle_queued_clicks(
        &self,
        commands: &mut UnboundedReceiver<PartyCommand>,
        running: &mut Option<Running>,
        lobby: &mut LobbyFollow,
    ) {
        let clicks = sort_queued_clicks(std::iter::from_fn(|| commands.try_recv().ok()));
        if clicks.ignored > 0 {
            info!(
                ignored = clicks.ignored,
                "Party clicks made while the join window was open were ignored"
            );
        }
        if clicks.leave {
            self.leave(running, lobby).await;
        }
    }

    async fn stop(&self, running: &mut Option<Running>) {
        if let Some(current) = running.take() {
            current.stop.cancel();

            let exit = current.task.await;
            debug!(?exit, "Party connection stopped");
        }
        set_party_share(&self.state_tx, false);
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

    fn start(
        &self,
        running: &mut Option<Running>,
        relay: String,
        token: PartyToken,
        room: Room,
    ) -> bool {
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
        set_party_share(&self.state_tx, room == Room::Code);
        *running = Some(Running {
            task: Box::pin(client.run(stop.clone())),
            stop,
            room,
        });
        true
    }

    async fn create(&self, running: &mut Option<Running>) {
        if self.created_dialog_open.load(Ordering::Acquire) {
            info!("The party room window is already open; no second room is created");
            return;
        }
        if in_code_room(running.as_ref()) {
            info!("Party create ignored: already in a room; leave it first");
            return;
        }
        let Some(relay) = self.relay_or_explain() else {
            return;
        };

        let token = match random_member_id().and_then(|host| PartyToken::generate(host, unix_now()))
        {
            Ok(token) => token,
            Err(e) => {
                self.explain_no_randomness(&e);
                return;
            }
        };
        let code = token.encode();
        self.hosted.remember(&token);

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

        self.stop(running).await;
        if self.start(running, relay, token, Room::Code) {
            set_party_hosting(&self.state_tx, true);
            show_created_dialog(code, Arc::clone(&self.created_dialog_open));
        }
    }

    async fn join(&self, running: &mut Option<Running>) {
        if in_code_room(running.as_ref()) {
            info!("Party join ignored: already in a room; leave it first");
            return;
        }
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
            .filter(|c| c.starts_with("DEKAN1:") && !self.hosted.is_own_code(c))
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
            Ok(token) if self.hosted.contains(&token) => {
                info!("Party join refused: the code is a room this Dekan created");
                notify(ui.party_join_title, ui.party_own_room.to_owned());
            }
            Ok(token) => {
                set_party_hosting(&self.state_tx, false);
                info!(
                    issued_at = token.issued_at,
                    "Joining a party room from an entered code"
                );
                self.stop(running).await;
                if self.start(running, relay, token, Room::Code) {
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

fn show_created_dialog(code: String, open: Arc<AtomicBool>) {
    open.store(true, Ordering::Release);
    drop(tokio::task::spawn_blocking(move || {
        let guard = DialogOpen(open);
        let shown = dekan_platform::party_dialog::show_party_created_dialog(&code);
        drop(guard);
        if let Err(e) = shown {
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

#[cfg(test)]
mod tests;
