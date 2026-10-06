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
    pub fn keeps_game_header(&self) -> bool {
        self.kept_checksum.is_some()
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

fn zstd_head(stored: &[u8]) -> Vec<u8> {
    let mut head = Vec::with_capacity(16);
    if let Ok(decoder) = zstd::Decoder::new(stored) {
        let _ = decoder.take(16).read_to_end(&mut head); // ignore-ok: a payload that does not decode is kept as stored; the magic check only needs what did decode
    }
    head
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
