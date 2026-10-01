use thiserror::Error;

#[derive(Debug, Error)]
pub enum WadError {
    #[error("invalid WAD magic: expected 'RW', got {0:?}")]
    InvalidMagic([u8; 2]),

    #[error("unsupported WAD version: {0}.{1}")]
    UnsupportedVersion(u8, u8),

    #[error("offset {offset} + size {size} exceeds buffer length {buffer_len}")]
    OffsetOutOfRange {
        offset: usize,
        size: usize,
        buffer_len: usize,
    },

    #[error("decompression failed: {0}")]
    Decompression(#[from] std::io::Error),

    #[error("entry {path_hash:#018x} decoded to {actual} bytes, TOC declares {declared}")]
    SizeMismatch {
        path_hash: u64,
        declared: usize,
        actual: usize,
    },

    #[error("invalid subchunk table: {0}")]
    InvalidSubchunkToc(String),

    #[error("unsupported compression type: {0}")]
    UnsupportedCompressionType(u8),

    #[error("checksum mismatch: expected {expected:#018x}, computed {computed:#018x}")]
    ChecksumMismatch { expected: u64, computed: u64 },

    #[error("invalid header size: buffer has {actual} bytes, expected at least {expected}")]
    InvalidHeaderSize { actual: usize, expected: usize },

    #[error("invalid PROP: {0}")]
    InvalidProp(String),

    #[error("invalid .fantome: {0}")]
    InvalidFantome(String),

    #[error("I/O error on '{path}': {source}")]
    FileIo {
        path: String,
        source: std::io::Error,
    },

    #[error("{what} is {value}, beyond what WAD v3.4 can store")]
    TooLarge { what: &'static str, value: u64 },

    /// A structure built by this crate contradicted itself (a layout missing an entry it listed).
    /// Never expected; reported instead of panicking.
    #[error("internal inconsistency: {0}")]
    Internal(&'static str),

    #[error("WAD write cancelled")]
    Cancelled,
}
