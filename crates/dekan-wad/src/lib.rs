#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod error;
pub mod fantome;
pub mod hash;
pub mod hash_index;
pub mod prop;
pub mod wad;
pub mod writer;

pub use wad::{CompressionType, WadArchive, WadEntry, WadFile, WadHeader};
pub use writer::{WadWriter, WriteOutcome, WriterEntry};
