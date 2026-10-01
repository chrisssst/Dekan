#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod champions;
pub mod env;
pub mod error;
pub mod forms;
pub mod historic;
pub mod library;
pub mod mods;
pub mod overlay;
pub mod party;
pub mod phase;
pub mod selection;
pub mod state;
pub mod supervisor;
