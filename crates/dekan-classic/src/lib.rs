#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod animation;
mod generation;

pub use animation::{clip_alias, forms, gear_toggle};
pub use generation::{builder, client_data, generator};

pub mod error;
