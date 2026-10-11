use std::io::Read;

use flate2::read::GzDecoder;

use tracing::{debug, warn};

use crate::error::WadError;

pub const WAD_HEADER_SIZE: usize = 272;

pub const WAD_ENTRY_SIZE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressionType {
    Raw = 0,

    Gzip = 1,

    Redirection = 2,

    Zstd = 3,

    ZstdChunked = 4,
}

impl CompressionType {
    pub fn from_type_byte(byte: u8) -> Result<Self, WadError> {
        match byte & 0x0F {
            0 => Ok(Self::Raw),
            1 => Ok(Self::Gzip),
            2 => Ok(Self::Redirection),
            3 => Ok(Self::Zstd),
            4 => Ok(Self::ZstdChunked),
            other => Err(WadError::UnsupportedCompressionType(other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WadEntry {
    pub path_hash: u64,

    pub offset: usize,

    pub compressed_size: usize,

    pub uncompressed_size: usize,

    pub compression: CompressionType,

    pub checksum: u64,

    pub subchunk_count: u8,

    pub first_subchunk: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WadHeader {
    pub major: u8,

    pub minor: u8,

    pub entry_count: usize,

    pub checksum: u64,
}

#[derive(Debug, Clone)]
pub struct WadArchive<'a> {
    data: &'a [u8],
    header: WadHeader,
    entries: Vec<WadEntry>,
    subchunk_toc: Option<SubchunkToc>,
}

impl<'a> WadArchive<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, WadError> {
        if data.len() < WAD_HEADER_SIZE {
            warn!(
                bytes = data.len(),
                expected = WAD_HEADER_SIZE,
                "WAD buffer is smaller than a v3 header"
            );
            return Err(WadError::InvalidHeaderSize {
                actual: data.len(),
                expected: WAD_HEADER_SIZE,
            });
        }

        let magic: [u8; 2] = [data[0], data[1]];
        if &magic != b"RW" {
            warn!(magic = ?magic, "Buffer is not a WAD archive (bad magic)");
            return Err(WadError::InvalidMagic(magic));
        }

        let major = data[2];
        let minor = data[3];
        if major != 3 {
            warn!(major, minor, "Unsupported WAD version");
            return Err(WadError::UnsupportedVersion(major, minor));
        }

        let checksum = u64::from_le_bytes(data[260..268].try_into().map_err(|_| {
            WadError::InvalidHeaderSize {
                actual: data.len(),
                expected: WAD_HEADER_SIZE,
            }
        })?);

        let entry_count = u32::from_le_bytes(data[268..272].try_into().map_err(|_| {
            WadError::InvalidHeaderSize {
                actual: data.len(),
                expected: WAD_HEADER_SIZE,
            }
        })?) as usize;

        let toc_size =
            entry_count
                .checked_mul(WAD_ENTRY_SIZE)
                .ok_or(WadError::OffsetOutOfRange {
                    offset: WAD_HEADER_SIZE,
                    size: usize::MAX,
                    buffer_len: data.len(),
                })?;

        let toc_end = WAD_HEADER_SIZE
            .checked_add(toc_size)
            .ok_or(WadError::OffsetOutOfRange {
                offset: WAD_HEADER_SIZE,
                size: toc_size,
                buffer_len: data.len(),
            })?;

        if data.len() < toc_end {
            return Err(WadError::OffsetOutOfRange {
                offset: WAD_HEADER_SIZE,
                size: toc_size,
                buffer_len: data.len(),
            });
        }

        let mut entries = Vec::with_capacity(entry_count);
        for i in 0..entry_count {
            let entry_offset = WAD_HEADER_SIZE + (i * WAD_ENTRY_SIZE);
            let chunk = &data[entry_offset..entry_offset + WAD_ENTRY_SIZE];
            entries.push(parse_toc_entry(chunk, entry_offset, data.len(), minor)?);
        }

        debug!(
            entries = entries.len(),
            declared_entries = entry_count,
            version = %format_args!("{major}.{minor}"),
            bytes = data.len(),
            "WAD parsed"
        );

        Ok(Self {
            data,
            header: WadHeader {
                major,
                minor,
                entry_count,
                checksum,
            },
            entries,
            subchunk_toc: None,
        })
    }

    pub fn load_subchunk_toc(&mut self, toc_name: &str) -> bool {
        let Some(entry) = self
            .find_by_hash(crate::hash::wad_path_hash(toc_name))
            .cloned()
        else {
            return false;
        };
        match self
            .read_entry(&entry)
            .and_then(|bytes| SubchunkToc::parse(&bytes))
        {
            Ok(toc) => {
                self.subchunk_toc = Some(toc);
                true
            }
            Err(e) => {
                warn!(toc = toc_name, error = %e, "WAD subchunk table unreadable");
                false
            }
        }
    }

    #[must_use]
    pub fn header(&self) -> &WadHeader {
        &self.header
    }

    #[must_use]
    pub fn entries(&self) -> &[WadEntry] {
        &self.entries
    }

    #[must_use]
    pub fn find_by_hash(&self, hash: u64) -> Option<&WadEntry> {
        self.entries.iter().find(|e| e.path_hash == hash)
    }

    pub fn raw_payload(&self, entry: &WadEntry) -> Result<&'a [u8], WadError> {
        let end =
            entry
                .offset
                .checked_add(entry.compressed_size)
                .ok_or(WadError::OffsetOutOfRange {
                    offset: entry.offset,
                    size: entry.compressed_size,
                    buffer_len: self.data.len(),
                })?;
        let raw_slice = self.data.get(entry.offset..end).ok_or_else(|| {
            warn!(
                path_hash = entry.path_hash,
                offset = entry.offset,
                size = entry.compressed_size,
                buffer_len = self.data.len(),
                "WAD entry payload lies outside the buffer"
            );
            WadError::OffsetOutOfRange {
                offset: entry.offset,
                size: entry.compressed_size,
                buffer_len: self.data.len(),
            }
        })?;
        Ok(raw_slice)
    }

    pub fn read_entry(&self, entry: &WadEntry) -> Result<Vec<u8>, WadError> {
        let raw_slice = self.raw_payload(entry)?;
        decompress_entry(entry, raw_slice, self.subchunk_toc.as_ref())
    }
}

fn parse_toc_entry(
    chunk: &[u8],
    entry_offset: usize,
    bound_len: usize,
    minor: u8,
) -> Result<WadEntry, WadError> {
    let field = |start: usize, len: usize| -> Result<&[u8], WadError> {
        chunk
            .get(start..start + len)
            .ok_or(WadError::OffsetOutOfRange {
                offset: entry_offset + start,
                size: len,
                buffer_len: bound_len,
            })
    };
    let read_u32 = |start: usize| -> Result<usize, WadError> {
        let bytes: [u8; 4] =
            field(start, 4)?
                .try_into()
                .map_err(|_| WadError::OffsetOutOfRange {
                    offset: entry_offset + start,
                    size: 4,
                    buffer_len: bound_len,
                })?;
        Ok(u32::from_le_bytes(bytes) as usize)
    };
    let read_u64 = |start: usize| -> Result<u64, WadError> {
        let bytes: [u8; 8] =
            field(start, 8)?
                .try_into()
                .map_err(|_| WadError::OffsetOutOfRange {
                    offset: entry_offset + start,
                    size: 8,
                    buffer_len: bound_len,
                })?;
        Ok(u64::from_le_bytes(bytes))
    };

    let path_hash = read_u64(0)?;
    let offset = read_u32(8)?;
    let compressed_size = read_u32(12)?;
    let uncompressed_size = read_u32(16)?;
    if uncompressed_size > MAX_ENTRY_BYTES {
        return Err(WadError::EntryTooLarge {
            path_hash,
            size: uncompressed_size,
        });
    }
    let type_byte = field(20, 1)?[0];
    let compression = CompressionType::from_type_byte(type_byte)?;
    let subchunk_count = type_byte >> 4;
    let index = field(21, 3)?;

    let first_subchunk = if minor >= 4 {
        (u32::from(index[0]) << 16) | u32::from(index[1]) | (u32::from(index[2]) << 8)
    } else {
        u32::from(u16::from_le_bytes([index[1], index[2]]))
    };
    let checksum = read_u64(24)?;

    let payload_end = offset
        .checked_add(compressed_size)
        .ok_or(WadError::OffsetOutOfRange {
            offset,
            size: compressed_size,
            buffer_len: bound_len,
        })?;
    if payload_end > bound_len {
        return Err(WadError::OffsetOutOfRange {
            offset,
            size: compressed_size,
            buffer_len: bound_len,
        });
    }

    Ok(WadEntry {
        path_hash,
        offset,
        compressed_size,
        uncompressed_size,
        compression,
        checksum,
        subchunk_count,
        first_subchunk,
    })
}

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

pub const MAX_ENTRY_BYTES: usize = 1 << 30;

mod decode;
mod file;

use decode::{MAX_PREALLOCATION, decompress_entry};
pub use decode::{SubchunkToc, subchunk_toc_name};
pub use file::{WAD_SIGNATURE_SIZE, WadFile};

#[cfg(test)]
mod tests;
