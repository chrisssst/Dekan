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
}

#[path = "prop_tree.rs"]
pub mod tree;

const FIELD_U32: u8 = 7;
const FIELD_STRING: u8 = 16;
const FIELD_LIST: u8 = 0x80;
const FIELD_LIST2: u8 = 0x81;
const FIELD_POINTER: u8 = 0x82;
const FIELD_EMBED: u8 = 0x83;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldValue<'a> {
    pub kind: u8,
    pub bytes: &'a [u8],
}

impl FieldValue<'_> {
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        matches!(self.kind, FIELD_U32 | 0x84 | 17 | 6)
            .then(|| self.bytes.get(..4))
            .flatten()
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

fn find_in_fields<'a>(
    cursor: &mut Cursor<'a>,
    path: &[u32],
) -> Result<Option<FieldValue<'a>>, WadError> {
    let Some((&wanted, rest)) = path.split_first() else {
        return Ok(None);
    };
    let count = cursor.u16("field count")?;
    for _ in 0..count {
        let name = cursor.u32("field name")?;
        let kind = cursor.take(1, "field type")?[0];
        let start = cursor.at;
        skip_field_value(cursor, kind)?;
        if name != wanted {
            continue;
        }
        let bytes = cursor
            .data
            .get(start..cursor.at)
            .ok_or_else(|| WadError::InvalidProp("field value out of range".into()))?;
        if rest.is_empty() {
            return Ok(Some(FieldValue { kind, bytes }));
        }
        if matches!(kind, FIELD_POINTER | FIELD_EMBED) && bytes.len() > 8 {
            let mut inner = Cursor { data: bytes, at: 8 };
            return find_in_fields(&mut inner, rest);
        }
        return Ok(None);
    }
    Ok(None)
}

const MAX_FIELD_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FlatField {
    pub path: String,
    pub kind: u8,
    pub hex: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn flatten_value(
    cursor: &mut Cursor<'_>,
    kind: u8,
    path: &str,
    depth: usize,
    out: &mut Vec<FlatField>,
) -> Result<(), WadError> {
    if depth > MAX_FIELD_DEPTH {
        return Err(WadError::InvalidProp(format!(
            "fields nested deeper than {MAX_FIELD_DEPTH}"
        )));
    }
    match kind {
        FIELD_LIST | FIELD_LIST2 => {
            let element = cursor.take(1, "list element type")?[0];
            cursor.u32("list size")?;
            let count = cursor.u32("list count")?;
            out.push(FlatField {
                path: format!("{path}.len"),
                kind,
                hex: format!("{count:08x}"),
            });
            for i in 0..count {
                flatten_value(cursor, element, &format!("{path}[{i}]"), depth + 1, out)?;
            }
        }
        FIELD_POINTER | FIELD_EMBED => {
            let class = cursor.u32("class")?;
            out.push(FlatField {
                path: format!("{path}.class"),
                kind,
                hex: format!("{class:08x}"),
            });
            if class != 0 {
                cursor.u32("struct size")?;
                flatten_fields_at(cursor, path, depth + 1, out)?;
            }
        }
        FIELD_OPTION => {
            let inner = cursor.take(1, "option type")?[0];
            let present = cursor.take(1, "option count")?[0];
            out.push(FlatField {
                path: format!("{path}.some"),
                kind,
                hex: format!("{present:02x}"),
            });
            if present != 0 {
                flatten_value(cursor, inner, &format!("{path}?"), depth + 1, out)?;
            }
        }
        FIELD_MAP => {
            let key_kind = cursor.take(1, "map key type")?[0];
            let value_kind = cursor.take(1, "map value type")?[0];
            cursor.u32("map size")?;
            let count = cursor.u32("map count")?;
            out.push(FlatField {
                path: format!("{path}.len"),
                kind,
                hex: format!("{count:08x}"),
            });
            for _ in 0..count {
                let start = cursor.at;
                skip_field_value(cursor, key_kind)?;
                let key = cursor
                    .data
                    .get(start..cursor.at)
                    .ok_or_else(|| WadError::InvalidProp("map key out of range".into()))?;
                flatten_value(
                    cursor,
                    value_kind,
                    &format!("{path}{{{}}}", hex(key)),
                    depth + 1,
                    out,
                )?;
            }
        }
        _ => {
            let start = cursor.at;
            skip_field_value(cursor, kind)?;
            let bytes = cursor
                .data
                .get(start..cursor.at)
                .ok_or_else(|| WadError::InvalidProp("field value out of range".into()))?;
            out.push(FlatField {
                path: path.to_owned(),
                kind,
                hex: hex(bytes),
            });
        }
    }
    Ok(())
}

fn flatten_fields_at(
    cursor: &mut Cursor<'_>,
    prefix: &str,
    depth: usize,
    out: &mut Vec<FlatField>,
) -> Result<(), WadError> {
    let count = cursor.u16("field count")?;
    for _ in 0..count {
        let name = cursor.u32("field name")?;
        let kind = cursor.take(1, "field type")?[0];
        let path = if prefix.is_empty() {
            format!("{name:08x}")
        } else {
            format!("{prefix}/{name:08x}")
        };
        flatten_value(cursor, kind, &path, depth, out)?;
    }
    Ok(())
}

pub fn flatten_fields(body: &[u8]) -> Result<Vec<FlatField>, WadError> {
    let mut out = Vec::new();
    flatten_fields_at(&mut Cursor { data: body, at: 0 }, "", 0, &mut out)?;
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FieldChange {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub fn diff_fields(before: &[u8], after: &[u8]) -> Result<Vec<FieldChange>, WadError> {
    let left: std::collections::BTreeMap<String, String> = flatten_fields(before)?
        .into_iter()
        .map(|f| (f.path, f.hex))
        .collect();
    let right: std::collections::BTreeMap<String, String> = flatten_fields(after)?
        .into_iter()
        .map(|f| (f.path, f.hex))
        .collect();
    let paths: std::collections::BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    Ok(paths
        .into_iter()
        .filter(|p| left.get(*p) != right.get(*p))
        .map(|p| FieldChange {
            path: p.clone(),
            before: left.get(p).cloned(),
            after: right.get(p).cloned(),
        })
        .collect())
}

const FIELD_HASH: u8 = 17;
const FIELD_LINK: u8 = 0x84;

fn reference_offsets_in_value(
    cursor: &mut Cursor<'_>,
    kind: u8,
    depth: usize,
    out: &mut Vec<usize>,
) -> Result<(), WadError> {
    if depth > MAX_FIELD_DEPTH {
        return Err(WadError::InvalidProp(format!(
            "fields nested deeper than {MAX_FIELD_DEPTH}"
        )));
    }
    match kind {
        FIELD_HASH | FIELD_LINK => {
            let at = cursor.at;
            cursor.take(4, "reference")?;
            out.push(at);
        }
        FIELD_LIST | FIELD_LIST2 => {
            let element = cursor.take(1, "list element type")?[0];
            cursor.u32("list size")?;
            let count = cursor.u32("list count")?;
            for _ in 0..count {
                reference_offsets_in_value(cursor, element, depth + 1, out)?;
            }
        }
        FIELD_POINTER | FIELD_EMBED => {
            if cursor.u32("class")? != 0 {
                cursor.u32("struct size")?;
                reference_offsets_in_fields(cursor, depth + 1, out)?;
            }
        }
        FIELD_OPTION => {
            let inner = cursor.take(1, "option type")?[0];
            if cursor.take(1, "option count")?[0] != 0 {
                reference_offsets_in_value(cursor, inner, depth + 1, out)?;
            }
        }
        FIELD_MAP => {
            let key_kind = cursor.take(1, "map key type")?[0];
            let value_kind = cursor.take(1, "map value type")?[0];
            cursor.u32("map size")?;
            let count = cursor.u32("map count")?;
            for _ in 0..count {
                if key_kind == FIELD_LINK {
                    out.push(cursor.at);
                }
                skip_field_value(cursor, key_kind)?;
                reference_offsets_in_value(cursor, value_kind, depth + 1, out)?;
            }
        }
        _ => skip_field_value(cursor, kind)?,
    }
    Ok(())
}

fn reference_offsets_in_fields(
    cursor: &mut Cursor<'_>,
    depth: usize,
    out: &mut Vec<usize>,
) -> Result<(), WadError> {
    let count = cursor.u16("field count")?;
    for _ in 0..count {
        cursor.u32("field name")?;
        let kind = cursor.take(1, "field type")?[0];
        reference_offsets_in_value(cursor, kind, depth, out)?;
    }
    Ok(())
}

fn reference_offsets(body: &[u8]) -> Result<Vec<usize>, WadError> {
    let mut out = Vec::new();
    reference_offsets_in_fields(&mut Cursor { data: body, at: 0 }, 0, &mut out)?;
    Ok(out)
}

fn u32_at(body: &[u8], at: usize) -> Result<u32, WadError> {
    body.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| WadError::InvalidProp(format!("truncated reference at offset {at}")))
}

pub fn reference_values(body: &[u8]) -> Result<Vec<u32>, WadError> {
    reference_offsets(body)?
        .into_iter()
        .map(|at| u32_at(body, at))
        .collect()
}

pub fn remap_references(
    body: &mut [u8],
    map: &std::collections::BTreeMap<u32, u32>,
) -> Result<usize, WadError> {
    let offsets = reference_offsets(body)?;
    let mut changed = 0;
    for at in offsets {
        let Some(&target) = map.get(&u32_at(body, at)?) else {
            continue;
        };
        let slot = body
            .get_mut(at..at + 4)
            .ok_or_else(|| WadError::InvalidProp(format!("truncated reference at offset {at}")))?;
        slot.copy_from_slice(&target.to_le_bytes());
        changed += 1;
    }
    Ok(changed)
}

pub fn field_value<'a>(body: &'a [u8], path: &[u32]) -> Result<Option<FieldValue<'a>>, WadError> {
    find_in_fields(&mut Cursor { data: body, at: 0 }, path)
}

pub fn set_u32_field(body: &mut [u8], field_hash: u32, value: u32) -> Result<bool, WadError> {
    set_top_level_int(body, field_hash, value, &[FIELD_U32])
}

pub fn set_int_field(body: &mut [u8], field_hash: u32, value: u32) -> Result<bool, WadError> {
    set_top_level_int(body, field_hash, value, &[FIELD_I32, FIELD_U32])
}

const FIELD_I32: u8 = 6;

fn set_top_level_int(
    body: &mut [u8],
    field_hash: u32,
    value: u32,
    kinds: &[u8],
) -> Result<bool, WadError> {
    let found = {
        let mut cursor = Cursor { data: body, at: 0 };
        let count = cursor.u16("field count")?;
        let mut found = None;
        for _ in 0..count {
            let name = cursor.u32("field name")?;
            let kind = cursor.take(1, "field type")?[0];
            if name == field_hash && kinds.contains(&kind) {
                found = Some(cursor.at);
                break;
            }
            skip_field_value(&mut cursor, kind)?;
        }
        found
    };
    let Some(at) = found else {
        return Ok(false);
    };
    let slot = body
        .get_mut(at..at + 4)
        .ok_or_else(|| WadError::InvalidProp(format!("truncated u32 field at offset {at}")))?;
    slot.copy_from_slice(&value.to_le_bytes());
    Ok(true)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prop_roundtrip_with_links() {
        let links = vec![
            "DATA/Characters/Annie/Skins/Skin0.bin".to_string(),
            "DATA/Characters/Annie/Animations/Skin0.bin".to_string(),
        ];

        let serialized = serialize_prop_links(&links, 3);
        let parsed = parse_prop_links(&serialized).expect("parse serialized prop");

        assert_eq!(links, parsed);
    }

    #[test]
    fn test_prop_with_ptch_prefix() {
        let links = vec!["DATA/Characters/Alistar/Skins/Skin0.bin".to_string()];
        let base_prop = serialize_prop_links(&links, 2);

        let mut ptch_data = Vec::new();
        ptch_data.extend_from_slice(PTCH_SIGNATURE);
        ptch_data.extend_from_slice(&1u32.to_le_bytes());
        ptch_data.extend_from_slice(&2u32.to_le_bytes());
        ptch_data.extend_from_slice(&base_prop);

        let header = parse_prop_header(&ptch_data).expect("parse ptch prop");
        assert!(header.has_ptch_header);
        assert_eq!(header.version, 2);
        assert_eq!(header.linked_files, links);
    }

    #[test]
    fn test_prop_empty_link_list() {
        let empty: Vec<String> = Vec::new();
        let serialized = serialize_prop_links(&empty, 3);
        let parsed = parse_prop_links(&serialized).expect("parse empty prop");
        assert!(parsed.is_empty());
    }

    fn sample_file() -> PropFile {
        PropFile {
            version: 3,
            links: vec!["DATA/Characters/Annie/Annie.bin".into()],
            entries: vec![
                PropEntry {
                    class_hash: 0x1111_1111,
                    key_hash: crate::hash::prop_key_hash("Characters/Annie/Skins/Skin1"),
                    body: vec![1, 2, 3, 4, 5],
                },
                PropEntry {
                    class_hash: 0x2222_2222,
                    key_hash: crate::hash::prop_key_hash("Characters/Annie/Skins/Skin1/Resources"),
                    body: vec![],
                },
            ],
        }
    }

    #[test]
    fn test_prop_file_roundtrip() {
        let file = sample_file();
        let bytes = serialize_prop_file(&file).expect("serialize");
        assert_eq!(parse_prop_file(&bytes).expect("parse"), file);
        let header = parse_prop_header(&bytes).expect("header");
        assert_eq!(header.linked_files, file.links);
        assert_eq!(header.entry_count, 2);
    }

    #[test]
    fn test_prop_file_accepts_ptch_and_rejects_damage() {
        let bytes = serialize_prop_file(&sample_file()).expect("serialize");
        let mut ptch = PTCH_SIGNATURE.to_vec();
        ptch.extend_from_slice(&[0u8; 8]);
        ptch.extend_from_slice(&bytes);
        assert_eq!(parse_prop_file(&ptch).expect("ptch"), sample_file());

        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(
            parse_prop_file(&trailing).is_err(),
            "trailing bytes are refused"
        );
        assert!(
            parse_prop_file(&bytes[..bytes.len() - 2]).is_err(),
            "truncation is refused"
        );

        let mut old = bytes.clone();
        old[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_prop_file(&old).is_err(), "version 1 is refused");
    }

    #[test]
    fn test_prop_truncated_error() {
        let data = b"PROP\x03\x00\x00\x00\x01\x00\x00\x00\x10\x00".to_vec();
        let err = parse_prop_links(&data).unwrap_err();
        assert!(matches!(err, WadError::InvalidProp(_)));
    }

    fn header_with_links() -> Vec<u8> {
        let mut data = b"PROP".to_vec();
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        for link in ["DATA/A.bin", "DATA/Characters/B/B.bin"] {
            data.extend_from_slice(&(link.len() as u16).to_le_bytes());
            data.extend_from_slice(link.as_bytes());
        }
        data.extend_from_slice(&7u32.to_le_bytes());
        data
    }

    #[test]
    fn test_a_header_that_ends_exactly_at_its_fields_parses() {
        let data = header_with_links();
        let header = parse_prop_header(&data).expect("exact length parses");
        assert_eq!(header.version, 3);
        assert_eq!(
            header.linked_files,
            vec!["DATA/A.bin", "DATA/Characters/B/B.bin"]
        );
        assert_eq!(header.entry_count, 7);
        assert!(!header.has_ptch_header);

        let mut ptch = b"PTCH".to_vec();
        ptch.extend_from_slice(&[0u8; 8]);
        ptch.extend_from_slice(&data);
        let patched = parse_prop_header(&ptch).expect("PTCH prefixed parses");
        assert!(patched.has_ptch_header);
        assert_eq!(patched.linked_files, header.linked_files);
        assert_eq!(patched.entry_count, 7);
    }

    #[test]
    fn test_every_truncation_of_a_header_is_an_error_or_the_same_links() {
        let data = header_with_links();
        let links_end = data.len() - 4;
        for cut in 0..data.len() {
            let result = parse_prop_header(&data[..cut]);
            if cut >= links_end {
                let header = result.unwrap_or_else(|e| panic!("cut {cut}: {e}"));
                assert_eq!(header.linked_files.len(), 2, "cut {cut}");
                assert_eq!(
                    header.entry_count, 0,
                    "cut {cut}: a partial entry count is not read"
                );
            } else {
                assert!(result.is_err(), "cut {cut} must be refused");
            }
        }
    }

    #[test]
    fn test_a_wrong_signature_is_refused() {
        let mut data = header_with_links();
        data[0] = b'X';
        assert!(parse_prop_header(&data).is_err());
        assert!(parse_prop_header(b"PTCH").is_err());
    }

    #[test]
    fn test_a_huge_link_count_is_an_error_not_a_huge_allocation() {
        let mut data = b"PROP\x03\x00\x00\x00".to_vec();
        data.extend_from_slice(&0xFF00_000Bu32.to_le_bytes());
        data.extend_from_slice(&[0x02, 0x00, b'a', b'b']);
        assert!(matches!(
            parse_prop_links(&data),
            Err(WadError::InvalidProp(_))
        ));
        let mut ptch = b"PTCH".to_vec();
        ptch.extend_from_slice(&[0u8; 8]);
        ptch.extend_from_slice(&data);
        assert!(parse_prop_header(&ptch).is_err());
    }

    fn field(body: &mut Vec<u8>, name: u32, kind: u8, value: &[u8]) {
        body.extend_from_slice(&name.to_le_bytes());
        body.push(kind);
        body.extend_from_slice(value);
    }

    fn body_with_target_after_every_container() -> Vec<u8> {
        let mut body = 8u16.to_le_bytes().to_vec();
        let mut string = 3u16.to_le_bytes().to_vec();
        string.extend_from_slice(b"abc");
        field(&mut body, 1, FIELD_STRING, &string);
        let mut list = vec![FIELD_U32];
        list.extend_from_slice(&12u32.to_le_bytes());
        list.extend_from_slice(&2u32.to_le_bytes());
        list.extend_from_slice(&[0xEE; 8]);
        field(&mut body, 2, FIELD_LIST, &list);
        let mut embed = 0xAAAA_AAAAu32.to_le_bytes().to_vec();
        embed.extend_from_slice(&8u32.to_le_bytes());
        embed.extend_from_slice(&1u16.to_le_bytes());
        embed.extend_from_slice(&0x8722_5880u32.to_le_bytes());
        embed.push(1);
        embed.push(1);
        field(&mut body, 3, FIELD_EMBED, &embed);
        field(&mut body, 4, FIELD_POINTER, &0u32.to_le_bytes());
        field(&mut body, 5, FIELD_OPTION, &[FIELD_U32, 1, 9, 9, 9, 9]);
        let mut map = vec![FIELD_U32, FIELD_U32];
        map.extend_from_slice(&12u32.to_le_bytes());
        map.extend_from_slice(&1u32.to_le_bytes());
        map.extend_from_slice(&[0xDD; 8]);
        field(&mut body, 6, FIELD_MAP, &map);
        field(&mut body, 7, 13, &[0xCC; 16]);
        field(&mut body, 0x8722_5880, FIELD_U32, &2u32.to_le_bytes());
        body
    }

    #[test]
    fn test_a_top_level_u32_field_is_found_after_every_container_kind() {
        let mut body = body_with_target_after_every_container();
        let original = body.clone();
        assert!(set_u32_field(&mut body, 0x8722_5880, 1).expect("set"));
        let len = body.len();
        assert_eq!(&body[len - 4..], &1u32.to_le_bytes());
        assert_eq!(
            &body[..len - 4],
            &original[..len - 4],
            "nothing else moves, not even the same hash nested inside the embed"
        );
    }

    #[test]
    fn test_a_missing_field_changes_nothing() {
        let mut body = body_with_target_after_every_container();
        let original = body.clone();
        assert!(!set_u32_field(&mut body, 0x1234_5678, 1).expect("walk"));
        assert_eq!(body, original);
    }

    #[test]
    fn test_every_truncation_of_a_body_is_an_error_or_leaves_it_untouched() {
        let full = body_with_target_after_every_container();
        for cut in 0..full.len() - 4 {
            let mut body = full[..cut].to_vec();
            let before = body.clone();
            match set_u32_field(&mut body, 0x8722_5880, 1) {
                Ok(changed) => assert!(!changed && body == before, "cut {cut}"),
                Err(WadError::InvalidProp(_)) => assert_eq!(body, before),
                Err(other) => panic!("cut {cut}: unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn test_a_field_is_found_by_path_through_an_embed() {
        let body = body_with_target_after_every_container();
        let top = field_value(&body, &[0x8722_5880])
            .expect("walk")
            .expect("top");
        assert_eq!(top.as_u32(), Some(2));
        let nested = field_value(&body, &[3, 0x8722_5880])
            .expect("walk")
            .expect("nested");
        assert_eq!((nested.kind, nested.bytes), (1, &[1u8][..]));
        assert_eq!(
            field_value(&body, &[4, 1]).expect("walk"),
            None,
            "a null pointer holds nothing"
        );
        assert_eq!(field_value(&body, &[0x1234]).expect("walk"), None);
        for cut in 0..body.len() {
            assert!(
                !matches!(field_value(&body[..cut], &[0x8722_5880]), Ok(Some(_))),
                "cut {cut} must not yield the last field"
            );
        }
    }

    #[test]
    fn test_every_field_is_flattened_through_every_container() {
        let body = body_with_target_after_every_container();
        let flat = flatten_fields(&body).expect("flatten");
        let paths: Vec<&str> = flat.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"00000001"), "string");
        assert!(
            paths.contains(&"00000002.len") && paths.contains(&"00000002[1]"),
            "list and its elements"
        );
        assert!(
            paths.contains(&"00000003.class") && paths.contains(&"00000003/87225880"),
            "embed body"
        );
        assert!(paths.contains(&"00000004.class"), "null pointer");
        assert!(
            paths.contains(&"00000005.some") && paths.contains(&"00000005?"),
            "option"
        );
        assert!(
            paths.contains(&"00000006.len")
                && paths.contains(
                    &"00000006{ffffffff}"
                        .replace("ffffffff", "dddddddd")
                        .as_str()
                ),
            "{paths:?}"
        );
        assert_eq!(
            flat.last().map(|f| (f.path.as_str(), f.hex.as_str())),
            Some(("87225880", "02000000"))
        );
    }

    #[test]
    fn test_a_diff_names_only_the_fields_that_changed() {
        let before = body_with_target_after_every_container();
        let mut after = before.clone();
        assert!(set_u32_field(&mut after, 0x8722_5880, 1).expect("set"));
        let changes = diff_fields(&before, &after).expect("diff");
        assert_eq!(
            changes,
            vec![FieldChange {
                path: "87225880".into(),
                before: Some("02000000".into()),
                after: Some("01000000".into()),
            }]
        );
        assert!(diff_fields(&before, &before).expect("same").is_empty());
    }

    #[test]
    fn test_flattening_a_hostile_body_is_an_error_not_a_panic() {
        let body = body_with_target_after_every_container();
        for cut in 0..body.len() {
            let _ = flatten_fields(&body[..cut]); // ignore-ok: only the absence of a panic is under test
        }
        let mut deep = 1u16.to_le_bytes().to_vec();
        deep.extend_from_slice(&1u32.to_le_bytes());
        deep.push(FIELD_OPTION);
        for _ in 0..80 {
            deep.extend_from_slice(&[FIELD_OPTION, 1]);
        }
        deep.extend_from_slice(&[FIELD_U32, 1, 7, 0, 0, 0]);
        assert!(flatten_fields(&deep).is_err(), "nesting is bounded");
    }

    const OLD: u32 = 0x50aa_299e;
    const NEW: u32 = 0x51aa_2b31;

    fn body_with_references_in_every_container() -> Vec<u8> {
        let mut body = 7u16.to_le_bytes().to_vec();
        field(&mut body, 1, FIELD_HASH, &OLD.to_le_bytes());
        field(&mut body, 2, FIELD_U32, &OLD.to_le_bytes());
        let mut list = vec![FIELD_LINK];
        list.extend_from_slice(&12u32.to_le_bytes());
        list.extend_from_slice(&2u32.to_le_bytes());
        list.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        list.extend_from_slice(&OLD.to_le_bytes());
        field(&mut body, 3, FIELD_LIST, &list);
        let mut embed = 0xAAAA_AAAAu32.to_le_bytes().to_vec();
        embed.extend_from_slice(&11u32.to_le_bytes());
        embed.extend_from_slice(&1u16.to_le_bytes());
        embed.extend_from_slice(&9u32.to_le_bytes());
        embed.push(FIELD_LINK);
        embed.extend_from_slice(&OLD.to_le_bytes());
        field(&mut body, 4, FIELD_EMBED, &embed);
        let mut option = vec![FIELD_HASH, 1];
        option.extend_from_slice(&OLD.to_le_bytes());
        field(&mut body, 5, FIELD_OPTION, &option);
        let mut map = vec![FIELD_LINK, FIELD_LINK];
        map.extend_from_slice(&12u32.to_le_bytes());
        map.extend_from_slice(&1u32.to_le_bytes());
        map.extend_from_slice(&OLD.to_le_bytes());
        map.extend_from_slice(&OLD.to_le_bytes());
        field(&mut body, 6, FIELD_MAP, &map);
        field(&mut body, 7, FIELD_STRING, &[2, 0, b'o', b'k']);
        body
    }

    #[test]
    fn test_every_reference_to_a_moved_key_follows_it_and_nothing_else_moves() {
        let mut body = body_with_references_in_every_container();
        let original = body.clone();
        let map = std::collections::BTreeMap::from([(OLD, NEW)]);
        assert_eq!(remap_references(&mut body, &map).expect("remap"), 6);
        assert_eq!(body.len(), original.len());
        let values = reference_values(&body).expect("references");
        assert!(
            !values.contains(&OLD),
            "no reference is left on the old key"
        );
        assert_eq!(values.iter().filter(|v| **v == NEW).count(), 6);
        assert!(
            values.contains(&0x1234_5678),
            "unrelated references are kept"
        );
        let plain = field_value(&body, &[2]).expect("walk").expect("u32");
        assert_eq!(
            plain.as_u32(),
            Some(OLD),
            "a plain u32 is data, not a reference"
        );
        assert_eq!(
            remap_references(&mut body, &map).expect("again"),
            0,
            "a second pass finds nothing"
        );
    }

    #[test]
    fn test_remapping_a_hostile_body_is_an_error_that_changes_nothing() {
        let full = body_with_references_in_every_container();
        let map = std::collections::BTreeMap::from([(OLD, NEW)]);
        for cut in 0..full.len() {
            let mut body = full[..cut].to_vec();
            let before = body.clone();
            if remap_references(&mut body, &map).is_err() {
                assert_eq!(body, before, "cut {cut}");
            }
        }
        let mut deep = 1u16.to_le_bytes().to_vec();
        deep.extend_from_slice(&1u32.to_le_bytes());
        deep.push(FIELD_OPTION);
        for _ in 0..80 {
            deep.extend_from_slice(&[FIELD_OPTION, 1]);
        }
        deep.extend_from_slice(&[FIELD_LINK, 1]);
        deep.extend_from_slice(&OLD.to_le_bytes());
        assert!(reference_values(&deep).is_err(), "nesting is bounded");
    }

    #[test]
    fn test_a_signed_int_field_is_set_only_by_the_int_setter() {
        let mut body = 2u16.to_le_bytes().to_vec();
        field(&mut body, 1, FIELD_I32, &5i32.to_le_bytes());
        field(&mut body, 2, FIELD_U32, &7u32.to_le_bytes());
        let original = body.clone();
        assert!(
            !set_u32_field(&mut body, 1, 0).expect("walk"),
            "u32 setter skips an i32"
        );
        assert_eq!(body, original);
        assert!(set_int_field(&mut body, 1, 0).expect("set i32"));
        assert!(set_int_field(&mut body, 2, 9).expect("set u32"));
        assert_eq!(
            field_value(&body, &[1])
                .expect("walk")
                .expect("i32")
                .as_u32(),
            Some(0)
        );
        assert_eq!(
            field_value(&body, &[2])
                .expect("walk")
                .expect("u32")
                .as_u32(),
            Some(9)
        );
    }

    #[test]
    fn test_an_unknown_field_type_is_refused() {
        let mut body = 1u16.to_le_bytes().to_vec();
        field(&mut body, 1, 0x7F, &[0; 4]);
        assert!(set_u32_field(&mut body, 0x8722_5880, 1).is_err());
    }
}
