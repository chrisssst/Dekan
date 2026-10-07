#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod build;
mod injector;

pub use build::{mod_compat, overlay, overlay_builder, overlay_cache};
pub use injector::{dll_validator, ltk_host, overlay_process, trust};

pub mod error;
pub mod pipeline;
