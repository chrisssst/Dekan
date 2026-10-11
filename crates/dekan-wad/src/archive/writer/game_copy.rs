use super::*;

const COPY_CHUNK: usize = 8 << 20;

#[must_use]
pub fn base_stamp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".base");
    PathBuf::from(name)
}

fn source_stamp(source: &Path, revision: &str) -> Result<String, WadError> {
    let meta = std::fs::metadata(source).map_err(|e| WadError::FileIo {
        path: source.display().to_string(),
        source: e,
    })?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Ok(format!("{}:{modified}:{revision}", meta.len()))
}

fn copy_cancellable(from: &Path, to: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), WadError> {
    let io = |p: &Path, source: std::io::Error| WadError::FileIo {
        path: p.display().to_string(),
        source,
    };
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
    }
    let partial = partial_path(to);
    let copied = (|| {
        let mut input = std::fs::File::open(from).map_err(|e| io(from, e))?;
        let mut output = std::fs::File::create(&partial).map_err(|e| io(&partial, e))?;
        let mut buffer = vec![0u8; COPY_CHUNK];
        loop {
            if cancelled() {
                return Err(WadError::Cancelled);
            }
            let read = input.read(&mut buffer).map_err(|e| io(from, e))?;
            if read == 0 {
                break;
            }
            output
                .write_all(&buffer[..read])
                .map_err(|e| io(&partial, e))?;
        }
        output.flush().map_err(|e| io(&partial, e))
    })();
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&partial); // ignore-ok: the copy error is what gets reported; a leftover partial is cleaned on the next build
        return Err(e);
    }
    std::fs::rename(&partial, to).map_err(|e| {
        let _ = std::fs::remove_file(&partial); // ignore-ok: the rename error is what gets reported
        io(to, e)
    })
}

pub fn ensure_game_copy(
    game_path: &Path,
    path: &Path,
    revision: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<bool, WadError> {
    let game_len = std::fs::metadata(game_path)
        .map_err(|e| WadError::FileIo {
            path: game_path.display().to_string(),
            source: e,
        })?
        .len();
    let stamp = source_stamp(game_path, revision)?;
    let stamp_path = base_stamp_path(path);
    let reusable = std::fs::read_to_string(&stamp_path).is_ok_and(|s| s == stamp)
        && std::fs::metadata(path).is_ok_and(|m| m.len() >= game_len);
    if reusable {
        return Ok(false);
    }
    let _ = std::fs::remove_file(&stamp_path); // ignore-ok: a stale stamp is rewritten below once the copy is complete
    copy_cancellable(game_path, path, cancelled)?;
    std::fs::write(&stamp_path, &stamp).map_err(|e| WadError::FileIo {
        path: stamp_path.display().to_string(),
        source: e,
    })?;
    Ok(true)
}

impl WadWriter {
    fn payload_bytes(&self, entry: &WriterEntry) -> Result<Vec<u8>, WadError> {
        match &entry.payload {
            Payload::Memory(bytes) => Ok(bytes.to_vec()),
            Payload::File {
                source,
                offset,
                len,
            } => {
                let path = self
                    .sources
                    .get(*source)
                    .ok_or(WadError::Internal("a source file"))?;
                let io = |source: std::io::Error| WadError::FileIo {
                    path: path.display().to_string(),
                    source,
                };
                let mut file = std::fs::File::open(path).map_err(io)?;
                file.seek(SeekFrom::Start(*offset)).map_err(io)?;
                let mut bytes = vec![0u8; *len];
                file.read_exact(&mut bytes).map_err(io)?;
                Ok(bytes)
            }
        }
    }

    pub fn write_over_game_copy(
        &self,
        path: &Path,
        revision: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<WriteOutcome>, WadError> {
        let Some(game_path) = self.sources.first() else {
            return Ok(None);
        };
        let io = |p: &Path| {
            let p = p.display().to_string();
            move |source: std::io::Error| WadError::FileIo {
                path: p.clone(),
                source,
            }
        };

        let mut game = std::fs::File::open(game_path).map_err(io(game_path))?;
        let game_len = game.metadata().map_err(io(game_path))?.len();
        let mut header = [0u8; WAD_HEADER_SIZE];
        game.read_exact(&mut header).map_err(io(game_path))?;
        if header[0..4] != VERSION {
            return Ok(None);
        }
        let count = u32::from_le_bytes([header[268], header[269], header[270], header[271]]);
        if count as usize != self.entries.len() {
            return Ok(None);
        }
        let mut toc = vec![0u8; WAD_ENTRY_SIZE * count as usize];
        game.read_exact(&mut toc).map_err(io(game_path))?;
        drop(game);

        let mut replaced: Vec<(usize, u64)> = Vec::new();
        for (index, raw) in toc.chunks_exact(WAD_ENTRY_SIZE).enumerate() {
            let name = u64::from_le_bytes(
                raw[0..8]
                    .try_into()
                    .map_err(|_| WadError::Internal("a TOC name"))?,
            );
            let offset = u64::from(u32::from_le_bytes(
                raw[8..12]
                    .try_into()
                    .map_err(|_| WadError::Internal("a TOC offset"))?,
            ));
            let stored = u32::from_le_bytes(
                raw[12..16]
                    .try_into()
                    .map_err(|_| WadError::Internal("a TOC size"))?,
            ) as usize;
            let Some(entry) = self.entries.get(&name) else {
                return Ok(None);
            };
            let unchanged = matches!(
                entry.payload,
                Payload::File { source: 0, offset: o, len } if o == offset && len == stored
            );
            if !unchanged {
                replaced.push((index, name));
            }
        }

        let reusable = !ensure_game_copy(game_path, path, revision, cancelled)?;

        let mut tail = Vec::new();
        let mut placed: HashMap<DedupKey, u64> = HashMap::new();
        for (index, name) in &replaced {
            let entry = self
                .entries
                .get(name)
                .ok_or(WadError::Internal("a replaced entry"))?;
            let key = entry.dedup_key();
            let offset = match placed.get(&key) {
                Some(&offset) => offset,
                None => {
                    let offset = game_len + tail.len() as u64;
                    tail.extend_from_slice(&self.payload_bytes(entry)?);
                    placed.insert(key, offset);
                    offset
                }
            };
            let raw = toc_entry(*name, entry, offset)?;
            let at = index * WAD_ENTRY_SIZE;
            toc.get_mut(at..at + WAD_ENTRY_SIZE)
                .ok_or(WadError::Internal("a TOC slot"))?
                .copy_from_slice(&raw);
        }
        let total = game_len + tail.len() as u64;

        if reusable && same_region(path, WAD_HEADER_SIZE as u64, &toc)? && {
            std::fs::metadata(path).is_ok_and(|m| m.len() == total)
                && same_region(path, game_len, &tail)?
        } {
            return Ok(Some(WriteOutcome::Unchanged { bytes: total }));
        }

        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(io(path))?;
        out.set_len(game_len).map_err(io(path))?;
        out.seek(SeekFrom::Start(game_len)).map_err(io(path))?;
        out.write_all(&tail).map_err(io(path))?;
        out.flush().map_err(io(path))?;
        out.seek(SeekFrom::Start(WAD_HEADER_SIZE as u64))
            .map_err(io(path))?;
        out.write_all(&toc).map_err(io(path))?;
        out.flush().map_err(io(path))?;
        Ok(Some(WriteOutcome::Written { bytes: total }))
    }
}

fn same_region(path: &Path, offset: u64, expected: &[u8]) -> Result<bool, WadError> {
    let io = |source: std::io::Error| WadError::FileIo {
        path: path.display().to_string(),
        source,
    };
    let mut file = std::fs::File::open(path).map_err(io)?;
    file.seek(SeekFrom::Start(offset)).map_err(io)?;
    let mut actual = vec![0u8; expected.len()];
    Ok(file.read_exact(&mut actual).is_ok() && actual == expected)
}
