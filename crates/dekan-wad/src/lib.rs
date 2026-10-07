#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod archive;
mod hashing;
mod properties;

pub use archive::{fantome, modpkg, wad, writer};
pub use hashing::{hash, hash_index};
pub use properties::prop;

pub mod error;

pub use wad::{CompressionType, WadArchive, WadEntry, WadFile, WadHeader};
pub use writer::{WadWriter, WriteOutcome, WriterEntry};
