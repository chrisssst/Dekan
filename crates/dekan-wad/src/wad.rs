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

        // 1. Magic check (offset 0..2: 'R', 'W')
        let magic: [u8; 2] = [data[0], data[1]];
        if &magic != b"RW" {
            warn!(magic = ?magic, "Buffer is not a WAD archive (bad magic)");
            return Err(WadError::InvalidMagic(magic));
        }

        let major = data[2];
        let minor = data[3];
        if major != 3 {
            // A new major version after a patch is exactly the kind of break that must be legible
            // in a log instead of surfacing as "the skin did not load".
            warn!(major, minor, "Unsupported WAD version");
            return Err(WadError::UnsupportedVersion(major, minor));
        }

        // Offset 260..268: checksum (u64 LE)
        let checksum = u64::from_le_bytes(data[260..268].try_into().map_err(|_| {
            WadError::InvalidHeaderSize {
                actual: data.len(),
                expected: WAD_HEADER_SIZE,
            }
        })?);

        // Offset 268..272: entry count (u32 LE)
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

        // 2. Parse TOC entries
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

    /// Load the archive's `.subchunktoc`, named after the WAD's path relative to the game folder
    /// (see [`subchunk_toc_name`]). Returns whether a table was found and loaded; without one,
    /// type-4 entries that start with a stored subchunk cannot be decoded.
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

    /// Access the parsed header.
    #[must_use]
    pub fn header(&self) -> &WadHeader {
        &self.header
    }

    /// Access the list of parsed entries.
    #[must_use]
    pub fn entries(&self) -> &[WadEntry] {
        &self.entries
    }

    /// Find an entry by path hash.
    #[must_use]
    pub fn find_by_hash(&self, hash: u64) -> Option<&WadEntry> {
        self.entries.iter().find(|e| e.path_hash == hash)
    }

    /// Access the raw (possibly compressed) payload slice of a WAD entry.
    ///
    /// Bounds-checked against the archive buffer: returns a typed error rather than panicking.
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubchunkToc {
    items: Vec<(u32, u32)>,
}

impl SubchunkToc {
    const ITEM_SIZE: usize = 16;

    pub fn parse(bytes: &[u8]) -> Result<Self, WadError> {
        if bytes.len() % Self::ITEM_SIZE != 0 {
            return Err(WadError::InvalidSubchunkToc(format!(
                "{} bytes is not a whole number of {}-byte items",
                bytes.len(),
                Self::ITEM_SIZE
            )));
        }
        let items = bytes
            .chunks_exact(Self::ITEM_SIZE)
            .map(|item| {
                (
                    u32::from_le_bytes([item[0], item[1], item[2], item[3]]),
                    u32::from_le_bytes([item[4], item[5], item[6], item[7]]),
                )
            })
            .collect();
        Ok(Self { items })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[must_use]
pub fn subchunk_toc_name(path: &std::path::Path) -> Option<String> {
    let lower = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let start = lower.find("data/final/")?;
    let relative = &lower[start..];
    let stem = relative.strip_suffix(".client").unwrap_or(relative);
    Some(format!("{stem}.subchunktoc"))
}

fn decode_with_toc(
    entry: &WadEntry,
    raw_slice: &[u8],
    toc: &SubchunkToc,
) -> Result<Vec<u8>, WadError> {
    let invalid = |what: String| {
        WadError::InvalidSubchunkToc(format!("entry {:#018x}: {what}", entry.path_hash))
    };
    let start = entry.first_subchunk as usize;
    let end = start
        .checked_add(usize::from(entry.subchunk_count))
        .ok_or_else(|| invalid("subchunk range overflows".into()))?;
    let items = toc.items.get(start..end).ok_or_else(|| {
        invalid(format!(
            "subchunks {start}..{end} outside a table of {}",
            toc.items.len()
        ))
    })?;

    let mut decoded = Vec::with_capacity(entry.uncompressed_size.min(MAX_PREALLOCATION));
    let mut position = 0usize;
    for &(stored, target) in items {
        let (stored, target) = (stored as usize, target as usize);
        let next = position
            .checked_add(stored)
            .ok_or_else(|| invalid("subchunk sizes overflow".into()))?;
        let chunk = raw_slice.get(position..next).ok_or_else(|| {
            invalid(format!(
                "subchunk ends at {next}, payload is {} bytes",
                raw_slice.len()
            ))
        })?;
        position = next;
        if stored == target {
            decoded.extend_from_slice(chunk);
        } else {
            let mut frame = Vec::with_capacity(target.min(MAX_PREALLOCATION));
            zstd::Decoder::new(chunk)?
                .take(target as u64 + 1)
                .read_to_end(&mut frame)?;
            if frame.len() != target {
                return Err(invalid(format!(
                    "subchunk decoded to {} bytes, table declares {target}",
                    frame.len()
                )));
            }
            decoded.extend_from_slice(&frame);
        }
    }
    if position != raw_slice.len() {
        return Err(invalid(format!(
            "subchunks cover {position} of {} payload bytes",
            raw_slice.len()
        )));
    }
    Ok(decoded)
}

fn decompress_entry(
    entry: &WadEntry,
    raw_slice: &[u8],
    toc: Option<&SubchunkToc>,
) -> Result<Vec<u8>, WadError> {
    let failed = |e: std::io::Error| -> WadError {
        warn!(
            path_hash = entry.path_hash,
            offset = entry.offset,
            compressed = entry.compressed_size,
            uncompressed = entry.uncompressed_size,
            compression = ?entry.compression,
            error = %e,
            "WAD entry could not be decompressed"
        );
        WadError::Decompression(e)
    };

    let decoded = match entry.compression {
        CompressionType::Redirection => return Ok(raw_slice.to_vec()),
        CompressionType::Raw => raw_slice.to_vec(),
        CompressionType::Gzip => read_bounded(GzDecoder::new(raw_slice), entry).map_err(failed)?,
        CompressionType::Zstd => {
            let decoder = zstd::Decoder::new(raw_slice).map_err(failed)?;
            read_bounded(decoder, entry).map_err(failed)?
        }

        CompressionType::ZstdChunked => {
            let streamed = if raw_slice.starts_with(&ZSTD_MAGIC) {
                zstd::Decoder::new(raw_slice).and_then(|decoder| read_bounded(decoder, entry))
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "payload starts with a stored subchunk",
                ))
            };
            match (streamed, toc) {
                (Ok(decoded), _) if decoded.len() == entry.uncompressed_size => decoded,
                (_, Some(toc)) if entry.subchunk_count > 0 => {
                    debug!(
                        path_hash = entry.path_hash,
                        subchunks = entry.subchunk_count,
                        "Type-4 entry decoded through the subchunk table"
                    );
                    decode_with_toc(entry, raw_slice, toc)?
                }
                (Ok(decoded), _) => decoded,
                (Err(e), _) => return Err(failed(e)),
            }
        }
    };

    if decoded.len() != entry.uncompressed_size {
        warn!(
            path_hash = entry.path_hash,
            compression = ?entry.compression,
            declared = entry.uncompressed_size,
            actual = decoded.len(),
            "WAD entry decoded to a size other than its TOC declares"
        );
        return Err(WadError::SizeMismatch {
            path_hash: entry.path_hash,
            declared: entry.uncompressed_size,
            actual: decoded.len(),
        });
    }
    Ok(decoded)
}

const MAX_PREALLOCATION: usize = 64 * 1024 * 1024;

fn read_bounded(reader: impl Read, entry: &WadEntry) -> std::io::Result<Vec<u8>> {
    let limit = (entry.uncompressed_size as u64).saturating_add(1);
    let mut decoded = Vec::with_capacity(entry.uncompressed_size.min(MAX_PREALLOCATION));
    reader.take(limit).read_to_end(&mut decoded)?;
    Ok(decoded)
}

#[derive(Debug)]
pub struct WadFile {
    path: std::path::PathBuf,
    entries: std::collections::HashMap<u64, WadEntry>,

    subchunk_toc: Option<SubchunkToc>,

    signature: [u8; WAD_SIGNATURE_SIZE],
    minor: u8,
}

pub const WAD_SIGNATURE_SIZE: usize = 256;

impl WadFile {
    pub fn open(path: &std::path::Path) -> Result<Self, WadError> {
        let mut wad = Self::open_toc_only(path)?;
        if let Some(name) = subchunk_toc_name(path) {
            match wad
                .read(crate::hash::wad_path_hash(&name))
                .and_then(|bytes| bytes.map(|b| SubchunkToc::parse(&b)).transpose())
            {
                Ok(toc) => wad.subchunk_toc = toc,
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "WAD subchunk table unreadable")
                }
            }
        }
        Ok(wad)
    }

    pub fn open_toc_only(path: &std::path::Path) -> Result<Self, WadError> {
        use std::io::Seek;

        let io = |source: std::io::Error| WadError::FileIo {
            path: path.display().to_string(),
            source,
        };
        let mut file = std::fs::File::open(path).map_err(io)?;
        let file_len = usize::try_from(file.metadata().map_err(io)?.len()).map_err(|_| {
            WadError::InvalidHeaderSize {
                actual: usize::MAX,
                expected: WAD_HEADER_SIZE,
            }
        })?;

        let mut header = [0u8; WAD_HEADER_SIZE];
        if file_len < WAD_HEADER_SIZE {
            return Err(WadError::InvalidHeaderSize {
                actual: file_len,
                expected: WAD_HEADER_SIZE,
            });
        }
        file.read_exact(&mut header).map_err(io)?;

        let magic: [u8; 2] = [header[0], header[1]];
        if &magic != b"RW" {
            return Err(WadError::InvalidMagic(magic));
        }
        if header[2] != 3 {
            return Err(WadError::UnsupportedVersion(header[2], header[3]));
        }

        let entry_count =
            u32::from_le_bytes([header[268], header[269], header[270], header[271]]) as usize;
        let toc_size =
            entry_count
                .checked_mul(WAD_ENTRY_SIZE)
                .ok_or(WadError::OffsetOutOfRange {
                    offset: WAD_HEADER_SIZE,
                    size: usize::MAX,
                    buffer_len: file_len,
                })?;

        if WAD_HEADER_SIZE.saturating_add(toc_size) > file_len {
            return Err(WadError::OffsetOutOfRange {
                offset: WAD_HEADER_SIZE,
                size: toc_size,
                buffer_len: file_len,
            });
        }

        file.seek(std::io::SeekFrom::Start(WAD_HEADER_SIZE as u64))
            .map_err(io)?;
        let mut toc = vec![0u8; toc_size];
        file.read_exact(&mut toc).map_err(io)?;

        let mut entries = std::collections::HashMap::with_capacity(entry_count);
        for (i, chunk) in toc.chunks_exact(WAD_ENTRY_SIZE).enumerate() {
            let entry = parse_toc_entry(
                chunk,
                WAD_HEADER_SIZE + i * WAD_ENTRY_SIZE,
                file_len,
                header[3],
            )?;
            entries.insert(entry.path_hash, entry);
        }

        debug!(
            path = %path.display(),
            entries = entries.len(),
            "WAD table of contents read from disk"
        );
        let mut signature = [0u8; WAD_SIGNATURE_SIZE];
        signature.copy_from_slice(&header[4..4 + WAD_SIGNATURE_SIZE]);
        Ok(Self {
            path: path.to_path_buf(),
            entries,
            subchunk_toc: None,
            signature,
            minor: header[3],
        })
    }

    #[must_use]
    pub fn signature(&self) -> &[u8; WAD_SIGNATURE_SIZE] {
        &self.signature
    }

    #[must_use]
    pub fn minor(&self) -> u8 {
        self.minor
    }

    #[must_use]
    pub fn entry(&self, path_hash: u64) -> Option<&WadEntry> {
        self.entries.get(&path_hash)
    }

    #[must_use]
    pub fn contains(&self, path_hash: u64) -> bool {
        self.entries.contains_key(&path_hash)
    }

    pub fn entries(&self) -> impl Iterator<Item = (u64, usize)> + '_ {
        self.entries
            .values()
            .map(|entry| (entry.path_hash, entry.uncompressed_size))
    }

    /// Every descriptor of the table of contents, in no particular order.
    pub fn toc(&self) -> impl Iterator<Item = &WadEntry> + '_ {
        self.entries.values()
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn read_raw(&self, entry: &WadEntry) -> Result<Vec<u8>, WadError> {
        use std::io::Seek;

        let io = |source: std::io::Error| WadError::FileIo {
            path: self.path.display().to_string(),
            source,
        };
        let mut file = std::fs::File::open(&self.path).map_err(io)?;
        file.seek(std::io::SeekFrom::Start(entry.offset as u64))
            .map_err(io)?;
        let mut raw = vec![0u8; entry.compressed_size];
        file.read_exact(&mut raw).map_err(io)?;
        Ok(raw)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn read(&self, path_hash: u64) -> Result<Option<Vec<u8>>, WadError> {
        use std::io::Seek;

        let Some(entry) = self.entries.get(&path_hash) else {
            return Ok(None);
        };
        let io = |source: std::io::Error| WadError::FileIo {
            path: self.path.display().to_string(),
            source,
        };
        let mut file = std::fs::File::open(&self.path).map_err(io)?;
        file.seek(std::io::SeekFrom::Start(entry.offset as u64))
            .map_err(io)?;
        let mut raw = vec![0u8; entry.compressed_size];
        file.read_exact(&mut raw).map_err(io)?;
        decompress_entry(entry, &raw, self.subchunk_toc.as_ref()).map(Some)
    }
}

#[cfg(test)]
#[path = "wad_tests.rs"]
mod tests;
