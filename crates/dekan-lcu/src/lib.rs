#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod champ_select;
pub mod champion_assets;
pub mod client;
pub mod error;
pub mod live_selection;
pub mod lockfile;
pub mod observer;
pub mod skin_registration;
pub mod websocket;
