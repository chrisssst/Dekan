use crate::error::WadError;
use crate::hash::wad_path_hash;

pub const HASH_INDEX_MAGIC: &[u8; 4] = b"BHSH";

pub const HASH_INDEX_VERSION: u32 = 1;

pub const HASH_INDEX_HEADER_SIZE: usize = 16;

pub const HASH_INDEX_ENTRY_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    pub hash: u64,
    pub str_offset: u32,
    pub str_len: u16,
}

#[derive(Debug, Clone)]
pub struct HashIndex<'a> {
    data: &'a [u8],
    entry_count: usize,
    string_pool_offset: usize,
}

impl<'a> HashIndex<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, WadError> {
        if data.len() < HASH_INDEX_HEADER_SIZE {
            return Err(WadError::OffsetOutOfRange {
                offset: 0,
                size: HASH_INDEX_HEADER_SIZE,
                buffer_len: data.len(),
            });
        }

        if &data[0..4] != HASH_INDEX_MAGIC {
            return Err(WadError::InvalidProp("invalid BHSH index magic".into()));
        }

        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != HASH_INDEX_VERSION {
            return Err(WadError::UnsupportedVersion(version as u8, 0));
        }

        let entry_count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let string_pool_offset =
            u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as usize;

        let table_size =
            entry_count
                .checked_mul(HASH_INDEX_ENTRY_SIZE)
                .ok_or(WadError::OffsetOutOfRange {
                    offset: HASH_INDEX_HEADER_SIZE,
                    size: usize::MAX,
                    buffer_len: data.len(),
                })?;

        let expected_pool_start = HASH_INDEX_HEADER_SIZE + table_size;
        if string_pool_offset < expected_pool_start || string_pool_offset > data.len() {
            return Err(WadError::OffsetOutOfRange {
                offset: expected_pool_start,
                size: 0,
                buffer_len: data.len(),
            });
        }

        Ok(Self {
            data,
            entry_count,
            string_pool_offset,
        })
    }

    /// Number of indexed path hashes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entry_count
    }

    /// Whether the index contains zero entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entry_count == 0
    }

    /// Read an entry record by its ordinal table index without allocation.
    fn read_entry(&self, idx: usize) -> IndexEntry {
        let offset = HASH_INDEX_HEADER_SIZE + (idx * HASH_INDEX_ENTRY_SIZE);
        let chunk = &self.data[offset..offset + HASH_INDEX_ENTRY_SIZE];

        let hash = u64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]);
        let str_offset = u32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
        let str_len = u16::from_le_bytes([chunk[12], chunk[13]]);

        IndexEntry {
            hash,
            str_offset,
            str_len,
        }
    }

    /// Find a file path matching the given 64-bit xxHash in O(log N) time.
    ///
    /// Returns a zero-copy string slice directly referencing the index buffer.
    #[must_use]
    pub fn find_path_by_hash(&self, target_hash: u64) -> Option<&'a str> {
        if self.entry_count == 0 {
            return None;
        }

        let mut low = 0;
        let mut high = self.entry_count;

        while low < high {
            let mid = low + (high - low) / 2;
            let entry = self.read_entry(mid);

            if entry.hash == target_hash {
                let str_start = self
                    .string_pool_offset
                    .checked_add(entry.str_offset as usize)?;
                let str_end = str_start.checked_add(entry.str_len as usize)?;

                if str_end <= self.data.len() {
                    let bytes = &self.data[str_start..str_end];
                    return std::str::from_utf8(bytes).ok();
                }
                return None;
            }

            if entry.hash < target_hash {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        None
    }

    #[must_use]
    pub fn find_path(&self, path: &str) -> Option<&'a str> {
        let hash = wad_path_hash(path);
        self.find_path_by_hash(hash)
    }
}

#[derive(Debug, Default)]
pub struct HashIndexBuilder {
    entries: Vec<(u64, String)>,
}

impl HashIndexBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn insert(&mut self, path: impl Into<String>) {
        let s = path.into();
        let hash = wad_path_hash(&s);
        self.entries.push((hash, s));
    }

    #[must_use]
    pub fn build(mut self) -> Vec<u8> {
        self.entries.sort_by_key(|&(h, _)| h);
        self.entries.dedup_by_key(|entry| entry.0);

        let entry_count = self.entries.len();
        let table_size = entry_count * HASH_INDEX_ENTRY_SIZE;
        let pool_offset = HASH_INDEX_HEADER_SIZE + table_size;

        let mut string_pool = Vec::new();
        let mut table_bytes = Vec::with_capacity(table_size);

        for (hash, path) in self.entries {
            let str_offset = string_pool.len() as u32;
            let path_bytes = path.as_bytes();
            let str_len = path_bytes.len() as u16;

            table_bytes.extend_from_slice(&hash.to_le_bytes());
            table_bytes.extend_from_slice(&str_offset.to_le_bytes());
            table_bytes.extend_from_slice(&str_len.to_le_bytes());
            table_bytes.extend_from_slice(&0u16.to_le_bytes());

            string_pool.extend_from_slice(path_bytes);
        }

        let mut output = Vec::with_capacity(pool_offset + string_pool.len());

        output.extend_from_slice(HASH_INDEX_MAGIC);
        output.extend_from_slice(&HASH_INDEX_VERSION.to_le_bytes());
        output.extend_from_slice(&(entry_count as u32).to_le_bytes());
        output.extend_from_slice(&(pool_offset as u32).to_le_bytes());

        output.extend_from_slice(&table_bytes);

        output.extend_from_slice(&string_pool);

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_index_builder_and_lookup() {
        let mut builder = HashIndexBuilder::new();
        builder.insert("DATA/Characters/Annie/Skins/Skin0.bin");
        builder.insert("DATA/Characters/Alistar/Skins/Skin0.bin");
        builder.insert("DATA/Characters/Vayne/Skins/Skin1.bin");

        let buffer = builder.build();
        let index = HashIndex::parse(&buffer).expect("parse index");

        assert_eq!(index.len(), 3);

        assert_eq!(
            index.find_path("DATA/Characters/Annie/Skins/Skin0.bin"),
            Some("DATA/Characters/Annie/Skins/Skin0.bin")
        );

        assert_eq!(
            index.find_path("data/characters/alistar/skins/skin0.bin"),
            Some("DATA/Characters/Alistar/Skins/Skin0.bin")
        );

        assert!(
            index
                .find_path("DATA/Characters/Zed/Skins/Skin0.bin")
                .is_none()
        );
    }

    #[test]
    fn test_empty_hash_index() {
        let builder = HashIndexBuilder::new();
        let buffer = builder.build();
        let index = HashIndex::parse(&buffer).expect("parse empty index");

        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert!(index.find_path("anything").is_none());
    }
}
