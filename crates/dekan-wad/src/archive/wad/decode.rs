use super::*;

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

pub(super) fn decompress_entry(
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

pub(super) const MAX_PREALLOCATION: usize = 64 * 1024 * 1024;

fn read_bounded(reader: impl Read, entry: &WadEntry) -> std::io::Result<Vec<u8>> {
    let limit = (entry.uncompressed_size as u64).saturating_add(1);
    let mut decoded = Vec::with_capacity(entry.uncompressed_size.min(MAX_PREALLOCATION));
    reader.take(limit).read_to_end(&mut decoded)?;
    Ok(decoded)
}
