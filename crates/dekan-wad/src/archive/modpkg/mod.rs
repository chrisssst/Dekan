use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use crate::error::WadError;
use crate::hash::content_checksum;
use crate::wad::CompressionType;
use crate::writer::{Payload, WriterEntry};

pub const MAGIC: &[u8; 8] = b"_modpkg_";

const VERSION: u32 = 1;
const NONE: u32 = u32::MAX;
const BASE_LAYER: &str = "base";
const MAX_NAME: u32 = 4096;
const MAX_TABLE: u32 = 1 << 20;
const MAX_CHUNK_BYTES: u64 = 512 * 1024 * 1024;
const WAD_SUFFIX: &str = ".wad.client";

#[must_use]
pub fn is_modpkg(head: &[u8]) -> bool {
    head.starts_with(MAGIC)
}

#[derive(Debug, Clone, Copy)]
struct Record {
    path_hash: u64,
    data_offset: u64,
    compression: u8,
    compressed_size: u64,
    uncompressed_size: u64,
    compressed_checksum: u64,
    layer: u32,
    wad: u32,
}

#[derive(Debug)]
pub struct ModPkg {
    wads: Vec<String>,
    base: u32,
    records: Vec<Record>,
}

fn invalid(why: impl Into<String>) -> WadError {
    WadError::InvalidModpkg(why.into())
}

fn io(e: std::io::Error) -> WadError {
    invalid(format!("truncated or unreadable: {e}"))
}

fn take<const N: usize>(reader: &mut impl Read) -> Result<[u8; N], WadError> {
    let mut bytes = [0u8; N];
    reader.read_exact(&mut bytes).map_err(io)?;
    Ok(bytes)
}

fn u32_le(reader: &mut impl Read) -> Result<u32, WadError> {
    take::<4>(reader).map(u32::from_le_bytes)
}

fn u64_le(reader: &mut impl Read) -> Result<u64, WadError> {
    take::<8>(reader).map(u64::from_le_bytes)
}

fn count(reader: &mut impl Read, what: &str) -> Result<u32, WadError> {
    let n = u32_le(reader)?;
    if n > MAX_TABLE {
        return Err(invalid(format!(
            "{n} {what} is more than a package can hold"
        )));
    }
    Ok(n)
}

fn counted_string(reader: &mut impl Read) -> Result<String, WadError> {
    let len = u32_le(reader)?;
    if len > MAX_NAME {
        return Err(invalid(format!("a {len}-byte name")));
    }
    let mut bytes = vec![0u8; len as usize];
    reader.read_exact(&mut bytes).map_err(io)?;
    String::from_utf8(bytes).map_err(|_| invalid("a name that is not UTF-8"))
}

fn nul_string(reader: &mut impl Read) -> Result<String, WadError> {
    let mut bytes = Vec::new();
    loop {
        match take::<1>(reader)?[0] {
            0 => break,
            byte if bytes.len() < MAX_NAME as usize => bytes.push(byte),
            _ => return Err(invalid("a name longer than allowed")),
        }
    }
    String::from_utf8(bytes).map_err(|_| invalid("a name that is not UTF-8"))
}

fn wad_stem(name: &str) -> Option<&str> {
    let file = name.rsplit(['/', '\\']).next()?;
    let stem = file.get(..file.len().checked_sub(WAD_SUFFIX.len())?)?;
    let valid = file[stem.len()..].eq_ignore_ascii_case(WAD_SUFFIX)
        && !stem.is_empty()
        && !stem.starts_with('.')
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._- ".contains(c));
    valid.then_some(stem)
}

impl ModPkg {
    pub fn read_index(mut reader: impl Read) -> Result<Self, WadError> {
        if &take::<8>(&mut reader)? != MAGIC {
            return Err(invalid("not a modpkg"));
        }
        let version = u32_le(&mut reader)?;
        if version != VERSION {
            return Err(invalid(format!(
                "format version {version} is not supported"
            )));
        }
        let signature = u64::from(u32_le(&mut reader)?);
        let chunk_count = count(&mut reader, "chunks")?;
        std::io::copy(&mut (&mut reader).take(signature), &mut std::io::sink()).map_err(io)?;
        let mut offset = 20 + signature;

        let mut base = None;
        for index in 0..count(&mut reader, "layers")? {
            let name = counted_string(&mut reader)?;
            take::<4>(&mut reader)?;
            offset += 8 + name.len() as u64;
            if name.eq_ignore_ascii_case(BASE_LAYER) {
                base = Some(index);
            }
        }
        let base = base.ok_or_else(|| invalid("no base layer"))?;
        offset += 4;

        for _ in 0..count(&mut reader, "paths")? {
            offset += nul_string(&mut reader)?.len() as u64 + 1;
        }
        offset += 4;

        let mut wads = Vec::new();
        for _ in 0..count(&mut reader, "WADs")? {
            let name = nul_string(&mut reader)?;
            offset += name.len() as u64 + 1;
            let stem = wad_stem(&name).ok_or_else(|| invalid(format!("WAD name '{name}'")))?;
            wads.push(stem.to_owned());
        }
        offset += 4;

        let padding = (8 - offset % 8) % 8;
        std::io::copy(&mut (&mut reader).take(padding), &mut std::io::sink()).map_err(io)?;

        let mut records = Vec::with_capacity((chunk_count as usize).min(4096));
        for _ in 0..chunk_count {
            let path_hash = u64_le(&mut reader)?;
            let data_offset = u64_le(&mut reader)?;
            let compression = take::<1>(&mut reader)?[0];
            let compressed_size = u64_le(&mut reader)?;
            let uncompressed_size = u64_le(&mut reader)?;
            let compressed_checksum = u64_le(&mut reader)?;
            u64_le(&mut reader)?;
            u32_le(&mut reader)?;
            let layer = u32_le(&mut reader)?;
            let wad = u32_le(&mut reader)?;
            if wad != NONE && wad as usize >= wads.len() {
                return Err(invalid(format!(
                    "a chunk points at WAD {wad} of {}",
                    wads.len()
                )));
            }
            records.push(Record {
                path_hash,
                data_offset,
                compression,
                compressed_size,
                uncompressed_size,
                compressed_checksum,
                layer,
                wad,
            });
        }
        Ok(Self {
            wads,
            base,
            records,
        })
    }

    #[must_use]
    pub fn wad_names(&self) -> BTreeSet<String> {
        self.base_records()
            .map(|record| self.wads[record.wad as usize].clone())
            .collect()
    }

    #[must_use]
    pub fn base_size(&self) -> u64 {
        self.base_records()
            .map(|record| record.uncompressed_size)
            .fold(0, u64::saturating_add)
    }

    fn base_records(&self) -> impl Iterator<Item = &Record> {
        self.records
            .iter()
            .filter(|record| record.layer == self.base && record.wad != NONE)
    }

    pub fn base_wads(
        &self,
        mut reader: impl Read + Seek,
    ) -> Result<BTreeMap<String, BTreeMap<u64, WriterEntry>>, WadError> {
        let file_len = reader.seek(SeekFrom::End(0)).map_err(io)?;
        let mut wads: BTreeMap<String, BTreeMap<u64, WriterEntry>> = BTreeMap::new();
        for record in self.base_records() {
            let end = record
                .data_offset
                .checked_add(record.compressed_size)
                .filter(|end| *end <= file_len)
                .ok_or_else(|| invalid("a chunk runs past the end of the file"))?;
            if record.compressed_size > MAX_CHUNK_BYTES
                || record.uncompressed_size > MAX_CHUNK_BYTES
            {
                return Err(invalid(format!(
                    "a {}-byte chunk is larger than allowed",
                    record.uncompressed_size.max(record.compressed_size)
                )));
            }
            reader
                .seek(SeekFrom::Start(record.data_offset))
                .map_err(io)?;
            let mut stored = vec![0u8; (end - record.data_offset) as usize];
            reader.read_exact(&mut stored).map_err(io)?;
            if content_checksum(&stored) != record.compressed_checksum {
                return Err(invalid(format!(
                    "chunk {:016x} does not match its checksum",
                    record.path_hash
                )));
            }
            let kind = match record.compression {
                0 if record.compressed_size == record.uncompressed_size => CompressionType::Raw,
                1 if zstd::zstd_safe::get_frame_content_size(&stored)
                    .ok()
                    .flatten()
                    .is_none_or(|size| size == record.uncompressed_size) =>
                {
                    CompressionType::Zstd
                }
                other => {
                    return Err(invalid(format!(
                        "chunk {:016x} with compression {other} and sizes {}/{}",
                        record.path_hash, record.compressed_size, record.uncompressed_size
                    )));
                }
            };
            wads.entry(self.wads[record.wad as usize].clone())
                .or_default()
                .insert(
                    record.path_hash,
                    WriterEntry {
                        kind: kind as u8,
                        subchunk_count: 0,
                        first_subchunk: 0,
                        uncompressed_size: record.uncompressed_size,
                        checksum: record.compressed_checksum,
                        payload: Payload::Memory(Arc::from(stored)),
                    },
                );
        }
        Ok(wads)
    }
}

#[cfg(test)]
mod tests;
