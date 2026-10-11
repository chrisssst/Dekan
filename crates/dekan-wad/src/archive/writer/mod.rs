use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use flate2::read::GzDecoder;
use tracing::debug;
use xxhash_rust::xxh3::Xxh3;

use crate::error::WadError;
use crate::hash::content_checksum;
use crate::wad::{CompressionType, WAD_ENTRY_SIZE, WAD_HEADER_SIZE, WAD_SIGNATURE_SIZE, WadEntry};

pub const VERSION: [u8; 4] = [b'R', b'W', 3, 4];

const ZSTD_LEVEL: i32 = 3;

const CANCEL_CHECK_EVERY: usize = 256;

#[derive(Debug, Clone)]
pub enum Payload {
    File {
        source: usize,
        offset: u64,
        len: usize,
    },

    Memory(Arc<[u8]>),
}

#[derive(Debug, Clone)]
pub struct WriterEntry {
    pub kind: u8,
    pub subchunk_count: u8,
    pub first_subchunk: u32,
    pub uncompressed_size: u64,

    pub checksum: u64,
    pub payload: Payload,
}

impl WriterEntry {
    #[must_use]
    pub fn from_wad(source: usize, entry: &WadEntry) -> Self {
        Self {
            kind: entry.compression as u8,
            subchunk_count: entry.subchunk_count,
            first_subchunk: entry.first_subchunk,
            uncompressed_size: entry.uncompressed_size as u64,
            checksum: entry.checksum,
            payload: Payload::File {
                source,
                offset: entry.offset as u64,
                len: entry.compressed_size,
            },
        }
    }

    fn stored_len(&self) -> usize {
        match &self.payload {
            Payload::File { len, .. } => *len,
            Payload::Memory(bytes) => bytes.len(),
        }
    }

    fn dedup_key(&self) -> DedupKey {
        match &self.payload {
            Payload::File {
                source,
                offset,
                len,
            } => DedupKey::Located {
                source: *source,
                offset: *offset,
                len: *len,
            },
            Payload::Memory(bytes) => DedupKey::Content {
                checksum: content_checksum(bytes),
                len: bytes.len(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DedupKey {
    Located {
        source: usize,
        offset: u64,
        len: usize,
    },

    Content {
        checksum: u64,
        len: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Unchanged { bytes: u64 },

    Written { bytes: u64 },
}

impl WriteOutcome {
    #[must_use]
    pub fn bytes(self) -> u64 {
        match self {
            Self::Unchanged { bytes } | Self::Written { bytes } => bytes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WadWriter {
    signature: [u8; WAD_SIGNATURE_SIZE],
    kept_checksum: Option<u64>,
    sources: Vec<PathBuf>,
    entries: BTreeMap<u64, WriterEntry>,
}

impl Default for WadWriter {
    fn default() -> Self {
        Self::new([0; WAD_SIGNATURE_SIZE])
    }
}

impl WadWriter {
    #[must_use]
    pub fn new(signature: [u8; WAD_SIGNATURE_SIZE]) -> Self {
        Self {
            signature,
            kept_checksum: None,
            sources: Vec::new(),
            entries: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn rebased_on(game: &crate::wad::WadFile) -> Self {
        Self {
            kept_checksum: Some(game.checksum()),
            ..Self::new(*game.signature())
        }
    }

    pub fn add_source(&mut self, path: &Path) -> usize {
        if let Some(i) = self.sources.iter().position(|p| p == path) {
            return i;
        }
        self.sources.push(path.to_path_buf());
        self.sources.len() - 1
    }

    pub fn insert(&mut self, path_hash: u64, entry: WriterEntry) {
        self.entries.insert(path_hash, entry);
    }

    #[must_use]
    pub fn contains(&self, path_hash: u64) -> bool {
        self.entries.contains_key(&path_hash)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries.keys().copied()
    }

    pub fn inserted(&self) -> impl Iterator<Item = (u64, &WriterEntry)> + '_ {
        self.entries
            .iter()
            .filter(|(_, entry)| !matches!(entry.payload, Payload::File { source: 0, .. }))
            .map(|(name, entry)| (*name, entry))
    }

    #[must_use]
    pub fn stored_len_of(entry: &WriterEntry) -> usize {
        entry.stored_len()
    }

    pub fn header(&self) -> Result<[u8; WAD_HEADER_SIZE], WadError> {
        let count = u32::try_from(self.entries.len()).map_err(|_| WadError::TooLarge {
            what: "entry count",
            value: self.entries.len() as u64,
        })?;
        let checksum = self.kept_checksum.unwrap_or_else(|| {
            let mut hasher = Xxh3::new();
            hasher.update(&VERSION);
            for (name, entry) in &self.entries {
                hasher.update(&name.to_le_bytes());
                hasher.update(&entry.checksum.to_le_bytes());
            }
            hasher.digest()
        });
        let mut header = [0u8; WAD_HEADER_SIZE];
        header[0..4].copy_from_slice(&VERSION);
        header[4..4 + WAD_SIGNATURE_SIZE].copy_from_slice(&self.signature);
        header[260..268].copy_from_slice(&checksum.to_le_bytes());
        header[268..272].copy_from_slice(&count.to_le_bytes());
        Ok(header)
    }

    fn layout(&self) -> Result<Layout, WadError> {
        let data_start = WAD_HEADER_SIZE + WAD_ENTRY_SIZE * self.entries.len();
        let mut order: Vec<(&u64, &WriterEntry)> = self.entries.iter().collect();
        order.sort_by_key(|(name, entry)| match &entry.payload {
            Payload::File { source, offset, .. } => (0u8, *source, *offset, **name),
            Payload::Memory(_) => (1, 0, 0, **name),
        });

        let mut placed: HashMap<DedupKey, u64> = HashMap::with_capacity(order.len());
        let mut writes = Vec::with_capacity(order.len());
        let mut offsets = HashMap::with_capacity(order.len());
        let mut cursor = data_start as u64;
        for (name, entry) in order {
            let key = entry.dedup_key();
            let offset = match placed.get(&key) {
                Some(&offset) => offset,
                None => {
                    let offset = cursor;
                    placed.insert(key, offset);
                    writes.push(*name);
                    cursor += entry.stored_len() as u64;
                    offset
                }
            };
            if offset > u64::from(u32::MAX) {
                return Err(WadError::TooLarge {
                    what: "entry offset",
                    value: offset,
                });
            }
            offsets.insert(*name, offset);
        }
        Ok(Layout {
            offsets,
            writes,
            total: cursor,
        })
    }
}

struct Layout {
    offsets: HashMap<u64, u64>,
    writes: Vec<u64>,
    total: u64,
}

#[must_use]
pub fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}

fn toc_entry(
    name: u64,
    entry: &WriterEntry,
    offset: u64,
) -> Result<[u8; WAD_ENTRY_SIZE], WadError> {
    let stored = u32::try_from(entry.stored_len()).map_err(|_| WadError::TooLarge {
        what: "stored entry size",
        value: entry.stored_len() as u64,
    })?;
    let decoded = u32::try_from(entry.uncompressed_size).map_err(|_| WadError::TooLarge {
        what: "decoded entry size",
        value: entry.uncompressed_size,
    })?;
    let offset = u32::try_from(offset).map_err(|_| WadError::TooLarge {
        what: "entry offset",
        value: offset,
    })?;
    let mut raw = [0u8; WAD_ENTRY_SIZE];
    raw[0..8].copy_from_slice(&name.to_le_bytes());
    raw[8..12].copy_from_slice(&offset.to_le_bytes());
    raw[12..16].copy_from_slice(&stored.to_le_bytes());
    raw[16..20].copy_from_slice(&decoded.to_le_bytes());
    raw[20] = ((entry.subchunk_count & 0x0F) << 4) | (entry.kind & 0x0F);
    let index = entry.first_subchunk;
    raw[21] = (index >> 16) as u8;
    raw[22] = index as u8;
    raw[23] = (index >> 8) as u8;
    raw[24..32].copy_from_slice(&entry.checksum.to_le_bytes());
    Ok(raw)
}

mod encode;
mod game_copy;
mod output;

pub use encode::{decode_zstd_bounded, is_audio_bank, optimal_raw, optimal_stored, prop_payload};
pub use game_copy::{base_stamp_path, ensure_game_copy};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod game_copy_tests;
