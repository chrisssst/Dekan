use super::*;

impl WadWriter {
    pub fn write_to_file(
        &self,
        path: &Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<WriteOutcome, WadError> {
        let io = |p: &Path| {
            let p = p.display().to_string();
            move |source: std::io::Error| WadError::FileIo {
                path: p.clone(),
                source,
            }
        };
        let layout = self.layout()?;
        let head = self.head(&layout)?;

        if let Ok(mut existing) = std::fs::File::open(path) {
            let mut old = vec![0u8; head.len()];
            let same_len = existing.metadata().is_ok_and(|m| m.len() == layout.total);
            if same_len && existing.read_exact(&mut old).is_ok() && old == head {
                debug!(path = %path.display(), bytes = layout.total, "WAD unchanged; not rewritten");
                return Ok(WriteOutcome::Unchanged {
                    bytes: layout.total,
                });
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let partial = partial_path(path);
        let result = self.write_partial(&partial, &head, &layout, cancelled);
        if let Err(e) = result {
            let _ = std::fs::remove_file(&partial); // ignore-ok: the write error is what gets reported; a leftover partial is cleaned on the next build
            return Err(e);
        }
        std::fs::rename(&partial, path).map_err(|e| {
            let _ = std::fs::remove_file(&partial); // ignore-ok: the rename error is what gets reported
            WadError::FileIo {
                path: path.display().to_string(),
                source: e,
            }
        })?;
        Ok(WriteOutcome::Written {
            bytes: layout.total,
        })
    }

    fn head(&self, layout: &Layout) -> Result<Vec<u8>, WadError> {
        let mut head = Vec::with_capacity(WAD_HEADER_SIZE + WAD_ENTRY_SIZE * self.entries.len());
        head.extend_from_slice(&self.header()?);
        for (name, entry) in &self.entries {
            let offset = layout
                .offsets
                .get(name)
                .copied()
                .ok_or(WadError::Internal("an entry offset"))?;
            head.extend_from_slice(&toc_entry(*name, entry, offset)?);
        }
        Ok(head)
    }

    fn write_partial(
        &self,
        partial: &Path,
        head: &[u8],
        layout: &Layout,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), WadError> {
        let io = |p: &Path| {
            let p = p.display().to_string();
            move |source: std::io::Error| WadError::FileIo {
                path: p.clone(),
                source,
            }
        };
        let file = std::fs::File::create(partial).map_err(io(partial))?;
        let mut out = std::io::BufWriter::with_capacity(1 << 20, file);

        let missing = WadError::Internal;

        out.write_all(head).map_err(io(partial))?;

        let mut copier = RunCopier::new(&self.sources);
        for (i, name) in layout.writes.iter().enumerate() {
            if i % CANCEL_CHECK_EVERY == 0 && cancelled() {
                return Err(WadError::Cancelled);
            }
            let entry = self.entries.get(name).ok_or(missing("an entry"))?;
            match &entry.payload {
                Payload::Memory(bytes) => {
                    copier.flush(&mut out, partial)?;
                    out.write_all(bytes).map_err(io(partial))?;
                }
                Payload::File {
                    source,
                    offset,
                    len,
                } => copier.push(&mut out, partial, *source, *offset, *len)?,
            }
        }
        copier.flush(&mut out, partial)?;

        out.flush().map_err(io(partial))?;
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, WadError> {
        static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("dekan_wadwriter_{}_{call}", std::process::id()));
        let path = dir.join("wad.wad.client");
        self.write_to_file(&path, &|| false)?;
        let bytes = std::fs::read(&path).map_err(|e| WadError::FileIo {
            path: path.display().to_string(),
            source: e,
        });
        let _ = std::fs::remove_dir_all(&dir); // ignore-ok: scratch folder of this call
        bytes
    }
}

const MAX_RUN: usize = 8 << 20;

struct RunCopier<'a> {
    sources: &'a [PathBuf],
    readers: HashMap<usize, std::fs::File>,
    buffer: Vec<u8>,

    run: Option<(usize, u64, usize)>,
}

impl<'a> RunCopier<'a> {
    fn new(sources: &'a [PathBuf]) -> Self {
        Self {
            sources,
            readers: HashMap::new(),
            buffer: Vec::new(),
            run: None,
        }
    }

    fn push(
        &mut self,
        out: &mut impl Write,
        partial: &Path,
        source: usize,
        offset: u64,
        len: usize,
    ) -> Result<(), WadError> {
        if let Some((run_source, start, run_len)) = self.run {
            if run_source == source && start + run_len as u64 == offset && run_len + len <= MAX_RUN
            {
                self.run = Some((run_source, start, run_len + len));
                return Ok(());
            }
            self.flush(out, partial)?;
        }
        self.run = Some((source, offset, len));
        Ok(())
    }

    fn flush(&mut self, out: &mut impl Write, partial: &Path) -> Result<(), WadError> {
        let Some((source, start, len)) = self.run.take() else {
            return Ok(());
        };
        let path = self
            .sources
            .get(source)
            .ok_or(WadError::Internal("a source file"))?;
        let io = |p: &Path, source: std::io::Error| WadError::FileIo {
            path: p.display().to_string(),
            source,
        };
        let reader = match self.readers.entry(source) {
            std::collections::hash_map::Entry::Occupied(slot) => slot.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(std::fs::File::open(path).map_err(|e| io(path, e))?)
            }
        };
        self.buffer.resize(len, 0);
        reader
            .seek(SeekFrom::Start(start))
            .map_err(|e| io(path, e))?;
        reader
            .read_exact(&mut self.buffer)
            .map_err(|e| io(path, e))?;
        out.write_all(&self.buffer).map_err(|e| io(partial, e))
    }
}
