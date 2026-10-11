use crate::error::WadError;

pub const PROP_SIGNATURE: &[u8; 4] = b"PROP";

pub const PTCH_SIGNATURE: &[u8; 4] = b"PTCH";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropHeader {
    pub has_ptch_header: bool,

    pub version: u32,

    pub linked_files: Vec<String>,

    pub entry_count: u32,
}

pub fn parse_prop_links(data: &[u8]) -> Result<Vec<String>, WadError> {
    let header = parse_prop_header(data)?;
    Ok(header.linked_files)
}

pub fn parse_prop_header(data: &[u8]) -> Result<PropHeader, WadError> {
    let mut cursor = Cursor { data, at: 0 };
    let has_ptch_header = data.get(0..4) == Some(PTCH_SIGNATURE.as_slice());
    if has_ptch_header {
        cursor.take(12, "PTCH header")?;
    }
    if cursor.take(4, "signature")? != PROP_SIGNATURE.as_slice() {
        return Err(WadError::InvalidProp(format!(
            "missing 'PROP' signature at offset {}",
            cursor.at - 4
        )));
    }
    let version = cursor.u32("version")?;
    let linked_count = cursor.u32("linked count")? as usize;

    let mut linked_files =
        Vec::with_capacity(linked_count.min(data.len().saturating_sub(cursor.at) / 2));
    for _ in 0..linked_count {
        let len = usize::from(cursor.u16("link string length")?);
        let raw = cursor.take(len, "link string payload")?;
        linked_files.push(
            String::from_utf8(raw.to_vec())
                .map_err(|e| WadError::InvalidProp(format!("invalid UTF-8 in link path: {e}")))?,
        );
    }

    let entry_count = if data.len() - cursor.at >= 4 {
        cursor.u32("entry count")?
    } else {
        0
    };

    Ok(PropHeader {
        has_ptch_header,
        version,
        linked_files,
        entry_count,
    })
}

#[must_use]
pub fn serialize_prop_links(links: &[String], version: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(PROP_SIGNATURE);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(links.len() as u32).to_le_bytes());

    for link in links {
        let bytes = link.as_bytes();
        out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(bytes);
    }

    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropEntry {
    pub class_hash: u32,

    pub key_hash: u32,

    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropFile {
    pub version: u32,
    pub links: Vec<String>,
    pub entries: Vec<PropEntry>,
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize, what: &str) -> Result<&'a [u8], WadError> {
        let end = self
            .at
            .checked_add(len)
            .ok_or_else(|| WadError::InvalidProp(format!("{what}: length overflow")))?;
        let slice = self.data.get(self.at..end).ok_or_else(|| {
            WadError::InvalidProp(format!("truncated {what} at offset {}", self.at))
        })?;
        self.at = end;
        Ok(slice)
    }

    fn u16(&mut self, what: &str) -> Result<u16, WadError> {
        let b = self.take(2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self, what: &str) -> Result<u32, WadError> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn count(&mut self, what: &str, item_kinds: &[u8]) -> Result<u32, WadError> {
        let count = self.u32(what)?;
        let remaining = self.data.len().saturating_sub(self.at);
        let item_size: usize = item_kinds
            .iter()
            .map(|kind| fixed_field_size(*kind).unwrap_or(1))
            .sum();
        let limit = remaining.checked_div(item_size).unwrap_or(MAX_EMPTY_ITEMS);
        if count as usize > limit {
            return Err(WadError::InvalidProp(format!(
                "{what} of {count} with {remaining} bytes left"
            )));
        }
        Ok(count)
    }
}

mod fields;
mod flatten;
mod refs;

pub use fields::{FieldValue, field_value, set_int_field};
pub use flatten::{FieldChange, FlatField, diff_fields, flatten_fields};
pub use refs::{reference_values, remap_references};

pub mod tree;

const FIELD_I32: u8 = 6;
const FIELD_U32: u8 = 7;
pub const FIELD_STRING: u8 = 16;
const FIELD_HASH: u8 = 17;
const FIELD_LIST: u8 = 0x80;
const FIELD_LIST2: u8 = 0x81;
const FIELD_POINTER: u8 = 0x82;
const FIELD_EMBED: u8 = 0x83;
const FIELD_LINK: u8 = 0x84;
const FIELD_OPTION: u8 = 0x85;
const FIELD_MAP: u8 = 0x86;

fn fixed_field_size(kind: u8) -> Option<usize> {
    Some(match kind {
        0 => 0,
        1..=3 | 0x87 => 1,
        4 | 5 => 2,
        6 | FIELD_U32 | 10 | 15 | 17 | 0x84 => 4,
        8 | 9 | 11 | 18 => 8,
        12 => 12,
        13 => 16,
        14 => 64,
        _ => return None,
    })
}

fn skip_field_value(cursor: &mut Cursor<'_>, kind: u8) -> Result<(), WadError> {
    if let Some(size) = fixed_field_size(kind) {
        cursor.take(size, "field value")?;
        return Ok(());
    }
    match kind {
        FIELD_STRING => {
            let len = cursor.u16("string length")? as usize;
            cursor.take(len, "string")?;
        }
        FIELD_LIST | FIELD_LIST2 => {
            cursor.take(1, "list element type")?;
            let size = cursor.u32("list size")? as usize;
            cursor.take(size, "list")?;
        }
        FIELD_POINTER | FIELD_EMBED => {
            if cursor.u32("class")? != 0 {
                let size = cursor.u32("struct size")? as usize;
                cursor.take(size, "struct")?;
            }
        }
        FIELD_OPTION => {
            let inner = cursor.take(1, "option type")?[0];
            if cursor.take(1, "option count")?[0] != 0 {
                skip_field_value(cursor, inner)?;
            }
        }
        FIELD_MAP => {
            cursor.take(2, "map types")?;
            let size = cursor.u32("map size")? as usize;
            cursor.take(size, "map")?;
        }
        other => {
            return Err(WadError::InvalidProp(format!(
                "unknown field type {other:#04x} at offset {}",
                cursor.at
            )));
        }
    }
    Ok(())
}

const MAX_FIELD_DEPTH: usize = 64;

const MAX_EMPTY_ITEMS: usize = u16::MAX as usize;

pub fn parse_prop_file(data: &[u8]) -> Result<PropFile, WadError> {
    let mut cursor = Cursor { data, at: 0 };
    if data.get(0..4) == Some(PTCH_SIGNATURE.as_slice()) {
        cursor.take(12, "PTCH header")?;
    }
    if cursor.take(4, "signature")? != PROP_SIGNATURE.as_slice() {
        return Err(WadError::InvalidProp("missing 'PROP' signature".into()));
    }
    let version = cursor.u32("version")?;
    if version < 2 {
        return Err(WadError::InvalidProp(format!(
            "unsupported PROP version {version}"
        )));
    }

    let link_count = cursor.u32("link count")? as usize;
    let mut links = Vec::with_capacity(link_count.min(4096));
    for _ in 0..link_count {
        let len = usize::from(cursor.u16("link length")?);
        let raw = cursor.take(len, "link")?;
        links.push(
            String::from_utf8(raw.to_vec())
                .map_err(|e| WadError::InvalidProp(format!("invalid UTF-8 in link path: {e}")))?,
        );
    }

    let entry_count = cursor.u32("entry count")? as usize;
    let types_len = entry_count
        .checked_mul(4)
        .ok_or_else(|| WadError::InvalidProp("entry count overflow".into()))?;
    let types = cursor.take(types_len, "type table")?;

    let mut entries = Vec::with_capacity(entry_count);
    for class in types.chunks_exact(4) {
        let class_hash = u32::from_le_bytes([class[0], class[1], class[2], class[3]]);
        let length = cursor.u32("object length")? as usize;
        if length < 4 {
            return Err(WadError::InvalidProp(format!(
                "object length {length} is shorter than its key"
            )));
        }
        let key_hash = cursor.u32("object key")?;
        let body = cursor.take(length - 4, "object body")?.to_vec();
        entries.push(PropEntry {
            class_hash,
            key_hash,
            body,
        });
    }

    if cursor.at != data.len() {
        return Err(WadError::InvalidProp(format!(
            "{} unexpected trailing bytes",
            data.len() - cursor.at
        )));
    }

    Ok(PropFile {
        version,
        links,
        entries,
    })
}

pub fn serialize_prop_file(file: &PropFile) -> Result<Vec<u8>, WadError> {
    let too_big = |what: &str| WadError::InvalidProp(format!("{what} does not fit its field"));
    let mut out = Vec::new();
    out.extend_from_slice(PROP_SIGNATURE);
    out.extend_from_slice(&file.version.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(file.links.len())
            .map_err(|_| too_big("link count"))?
            .to_le_bytes(),
    );
    for link in &file.links {
        let bytes = link.as_bytes();
        out.extend_from_slice(
            &u16::try_from(bytes.len())
                .map_err(|_| too_big("link"))?
                .to_le_bytes(),
        );
        out.extend_from_slice(bytes);
    }
    out.extend_from_slice(
        &u32::try_from(file.entries.len())
            .map_err(|_| too_big("entry count"))?
            .to_le_bytes(),
    );
    for entry in &file.entries {
        out.extend_from_slice(&entry.class_hash.to_le_bytes());
    }
    for entry in &file.entries {
        let length = u32::try_from(entry.body.len() + 4).map_err(|_| too_big("object"))?;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&entry.key_hash.to_le_bytes());
        out.extend_from_slice(&entry.body);
    }
    Ok(out)
}

#[must_use]
pub fn is_prop(data: &[u8]) -> bool {
    matches!(data.get(0..4), Some(magic) if magic == PROP_SIGNATURE || magic == PTCH_SIGNATURE)
}

pub fn record_field_shapes(data: &[u8], shapes: &mut tree::FieldShapes) -> Result<(), WadError> {
    for entry in parse_prop_file(data)?.entries {
        shapes.record(entry.class_hash, &tree::parse_fields(&entry.body)?);
    }
    Ok(())
}

pub fn strings_to_files(
    data: &[u8],
    shapes: &tree::FieldShapes,
) -> Result<Option<(Vec<u8>, usize)>, WadError> {
    if data.get(0..4) != Some(PROP_SIGNATURE.as_slice()) {
        return Ok(None);
    }
    let mut file = parse_prop_file(data)?;
    let mut retyped = 0;
    for entry in &mut file.entries {
        let mut fields = tree::parse_fields(&entry.body)?;
        let changed = shapes.strings_to_files(entry.class_hash, &mut fields);
        if changed > 0 {
            entry.body = tree::write_fields(&fields)?;
            retyped += changed;
        }
    }
    if retyped == 0 {
        return Ok(None);
    }
    Ok(Some((serialize_prop_file(&file)?, retyped)))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod body_tests;
