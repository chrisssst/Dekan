#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod connection;
mod session;

pub use connection::{client, lockfile, observer, websocket};
pub use session::{champ_select, champion_assets, live_selection, lobby, skin_registration};

pub mod error;
