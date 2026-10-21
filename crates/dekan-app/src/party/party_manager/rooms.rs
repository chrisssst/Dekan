use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use dekan_party::token::{PartyToken, unix_now};

use super::PartyCommand;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueuedClicks {
    pub ignored: usize,
    pub leave: bool,
}

#[derive(Debug, Default)]
pub struct HostedRooms(Mutex<Vec<(u64, u64)>>);

impl HostedRooms {
    pub fn remember(&self, token: &PartyToken) {
        if let Ok(mut rooms) = self.0.lock() {
            rooms.push((token.host_member, token.issued_at));
        }
    }

    #[must_use]
    pub fn contains(&self, token: &PartyToken) -> bool {
        self.0
            .lock()
            .is_ok_and(|rooms| rooms.contains(&(token.host_member, token.issued_at)))
    }

    #[must_use]
    pub fn is_own_code(&self, code: &str) -> bool {
        PartyToken::decode(code, unix_now()).is_ok_and(|token| self.contains(&token))
    }
}

#[must_use]
pub fn sort_queued_clicks(commands: impl IntoIterator<Item = PartyCommand>) -> QueuedClicks {
    commands
        .into_iter()
        .fold(QueuedClicks::default(), |mut clicks, command| {
            match command {
                PartyCommand::Leave => clicks.leave = true,
                PartyCommand::Create | PartyCommand::Join => clicks.ignored += 1,
            }
            clicks
        })
}

pub(super) struct DialogOpen(pub(super) Arc<AtomicBool>);

impl Drop for DialogOpen {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
