#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod builder;
pub mod client_data;
pub mod clip_alias;
pub mod error;
pub mod forms;
pub mod gear_toggle;
pub mod generator;
