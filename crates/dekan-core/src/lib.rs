#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod domain;
mod runtime;

pub use domain::{historic, library, lobby, mods, overlay, party, presets};
pub use runtime::{phase, selection, state, supervisor};

pub mod env;
pub mod error;
