use super::*;

struct AtReader<'a> {
    file: &'a std::fs::File,
    position: u64,
}

impl Read for AtReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = read_at(self.file, buf, self.position)?;
        self.position += read as u64;
        Ok(read)
    }
}

#[cfg(windows)]
fn read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(unix)]
fn read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

fn read_exact_at(file: &std::fs::File, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    AtReader {
        file,
        position: offset,
    }
    .read_exact(&mut buf)?;
    Ok(buf)
}

#[derive(Debug)]
pub struct WadFile {
    path: std::path::PathBuf,
    file: std::fs::File,
    entries: std::collections::HashMap<u64, WadEntry>,

    subchunk_toc: Option<SubchunkToc>,

    signature: [u8; WAD_SIGNATURE_SIZE],
    checksum: u64,
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
        let mut checksum = [0u8; 8];
        checksum.copy_from_slice(&header[260..268]);
        Ok(Self {
            path: path.to_path_buf(),
            file,
            entries,
            subchunk_toc: None,
            signature,
            checksum: u64::from_le_bytes(checksum),
            minor: header[3],
        })
    }

    #[must_use]
    pub fn signature(&self) -> &[u8; WAD_SIGNATURE_SIZE] {
        &self.signature
    }

    #[must_use]
    pub fn checksum(&self) -> u64 {
        self.checksum
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

    pub fn toc(&self) -> impl Iterator<Item = &WadEntry> + '_ {
        self.entries.values()
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn read_raw(&self, entry: &WadEntry) -> Result<Vec<u8>, WadError> {
        read_exact_at(&self.file, entry.offset as u64, entry.compressed_size).map_err(|source| {
            WadError::FileIo {
                path: self.path.display().to_string(),
                source,
            }
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn read_prefix(&self, path_hash: u64, len: usize) -> Result<Option<Vec<u8>>, WadError> {
        let Some(entry) = self.entries.get(&path_hash) else {
            return Ok(None);
        };
        let io = |source: std::io::Error| WadError::FileIo {
            path: self.path.display().to_string(),
            source,
        };
        let at = AtReader {
            file: &self.file,
            position: entry.offset as u64,
        };
        let payload = std::io::BufReader::new(at.take(entry.compressed_size as u64));
        let wanted = len.min(entry.uncompressed_size) as u64;
        let mut prefix = Vec::with_capacity(len.min(MAX_PREALLOCATION));
        let read = match entry.compression {
            CompressionType::Redirection | CompressionType::Raw => {
                payload.take(wanted).read_to_end(&mut prefix)
            }
            CompressionType::Gzip => GzDecoder::new(payload)
                .take(wanted)
                .read_to_end(&mut prefix),
            CompressionType::Zstd | CompressionType::ZstdChunked => {
                let mut payload = payload;
                let starts_compressed = std::io::BufRead::fill_buf(&mut payload)
                    .map(|head| head.starts_with(&ZSTD_MAGIC))
                    .map_err(io)?;
                if starts_compressed {
                    zstd::Decoder::with_buffer(payload)
                        .and_then(|decoder| decoder.take(wanted).read_to_end(&mut prefix))
                } else {
                    payload.take(wanted).read_to_end(&mut prefix)
                }
            }
        };
        read.map_err(WadError::Decompression)?;
        Ok(Some(prefix))
    }

    pub fn read(&self, path_hash: u64) -> Result<Option<Vec<u8>>, WadError> {
        let Some(entry) = self.entries.get(&path_hash) else {
            return Ok(None);
        };
        let raw = self.read_raw(entry)?;
        decompress_entry(entry, &raw, self.subchunk_toc.as_ref()).map(Some)
    }
}
