use super::*;

#[must_use]
pub fn is_audio_bank(head: &[u8]) -> bool {
    const NOT_AUDIO: [&[u8]; 7] = [
        b"r3d2Mesh",
        b"r3d2aims",
        b"r3d2anmd",
        b"r3d2canm",
        b"r3d2sklt",
        b"r3d2blnd",
        b"r3d2wght",
    ];
    head.starts_with(b"BKHD")
        || (head.starts_with(b"r3d2") && !NOT_AUDIO.iter().any(|m| head.starts_with(m)))
}

pub fn optimal_raw(decoded: Vec<u8>) -> Result<WriterEntry, WadError> {
    let uncompressed_size = decoded.len() as u64;
    let (kind, stored) = if is_audio_bank(&decoded) {
        (CompressionType::Raw as u8, decoded)
    } else {
        (
            CompressionType::Zstd as u8,
            zstd::bulk::compress(&decoded, ZSTD_LEVEL).map_err(WadError::Decompression)?,
        )
    };
    Ok(WriterEntry {
        kind,
        subchunk_count: 0,
        first_subchunk: 0,
        uncompressed_size,
        checksum: content_checksum(&stored),
        payload: Payload::Memory(Arc::from(stored)),
    })
}

pub fn optimal_stored(
    entry: &WadEntry,
    stored: Vec<u8>,
    decode: impl FnOnce() -> Result<Vec<u8>, WadError>,
) -> Result<WriterEntry, WadError> {
    const GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];
    let keep = |stored: Vec<u8>| {
        let checksum = if entry.checksum != 0 {
            entry.checksum
        } else {
            content_checksum(&stored)
        };
        WriterEntry {
            kind: entry.compression as u8,
            subchunk_count: 0,
            first_subchunk: 0,
            uncompressed_size: entry.uncompressed_size as u64,
            checksum,
            payload: Payload::Memory(Arc::from(stored)),
        }
    };

    match entry.compression {
        CompressionType::Raw => optimal_raw(stored),
        CompressionType::Zstd => {
            let head = zstd_head(&stored);
            if is_audio_bank(&head) {
                optimal_raw(decode()?)
            } else {
                Ok(keep(stored))
            }
        }
        CompressionType::ZstdChunked => optimal_raw(decode()?),
        CompressionType::Gzip | CompressionType::Redirection => {
            if stored.starts_with(&GZIP_MAGIC) {
                let mut decoded = Vec::new();
                GzDecoder::new(stored.as_slice())
                    .take(entry.uncompressed_size as u64 + 1)
                    .read_to_end(&mut decoded)?;
                if decoded.len() != entry.uncompressed_size {
                    return Err(WadError::SizeMismatch {
                        path_hash: entry.path_hash,
                        declared: entry.uncompressed_size,
                        actual: decoded.len(),
                    });
                }
                optimal_raw(decoded)
            } else {
                Ok(keep(stored))
            }
        }
    }
}

#[must_use]
pub fn prop_payload(entry: &WriterEntry) -> Option<Vec<u8>> {
    let Payload::Memory(stored) = &entry.payload else {
        return None;
    };
    if entry.subchunk_count != 0 {
        return None;
    }
    if entry.kind == CompressionType::Raw as u8 {
        return crate::prop::is_prop(stored).then(|| stored.to_vec());
    }
    if entry.kind != CompressionType::Zstd as u8 || !crate::prop::is_prop(&zstd_head(stored)) {
        return None;
    }
    decode_zstd_bounded(stored, entry.uncompressed_size)
}

#[must_use]
pub fn decode_zstd_bounded(stored: &[u8], declared: u64) -> Option<Vec<u8>> {
    let limit = declared.min(crate::wad::MAX_ENTRY_BYTES as u64);
    let mut decoded = Vec::new();
    zstd::Decoder::new(stored)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut decoded)
        .ok()?;
    (decoded.len() as u64 == declared).then_some(decoded)
}

fn zstd_head(stored: &[u8]) -> Vec<u8> {
    let mut head = Vec::with_capacity(16);
    if let Ok(decoder) = zstd::Decoder::new(stored) {
        let _ = decoder.take(16).read_to_end(&mut head); // ignore-ok: a payload that does not decode is kept as stored; the magic check only needs what did decode
    }
    head
}
