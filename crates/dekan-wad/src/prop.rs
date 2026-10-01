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
    if data.len() < 4 {
        return Err(WadError::InvalidProp("data smaller than 4 bytes".into()));
    }

    let mut cursor = 0;
    let mut has_ptch_header = false;

    if &data[0..4] == PTCH_SIGNATURE {
        if data.len() < 16 {
            return Err(WadError::InvalidProp(
                "PTCH binary smaller than 16 bytes".into(),
            ));
        }
        has_ptch_header = true;
        cursor = 12;
    }

    if data.len() < cursor + 4 || &data[cursor..cursor + 4] != PROP_SIGNATURE {
        return Err(WadError::InvalidProp(format!(
            "missing 'PROP' signature at offset {cursor}"
        )));
    }
    cursor += 4;

    if data.len() < cursor + 4 {
        return Err(WadError::InvalidProp("truncated version field".into()));
    }
    let version = u32::from_le_bytes([
        data[cursor],
        data[cursor + 1],
        data[cursor + 2],
        data[cursor + 3],
    ]);
    cursor += 4;

    if data.len() < cursor + 4 {
        return Err(WadError::InvalidProp("truncated linked count field".into()));
    }
    let linked_count = u32::from_le_bytes([
        data[cursor],
        data[cursor + 1],
        data[cursor + 2],
        data[cursor + 3],
    ]) as usize;
    cursor += 4;

    let mut linked_files = Vec::with_capacity(linked_count);
    for _ in 0..linked_count {
        if data.len() < cursor + 2 {
            return Err(WadError::InvalidProp("truncated link string length".into()));
        }
        let str_len = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
        cursor += 2;

        if data.len() < cursor + str_len {
            return Err(WadError::InvalidProp(
                "truncated link string payload".into(),
            ));
        }
        let raw_str = &data[cursor..cursor + str_len];
        let link_str = String::from_utf8(raw_str.to_vec())
            .map_err(|e| WadError::InvalidProp(format!("invalid UTF-8 in link path: {e}")))?;
        cursor += str_len;

        linked_files.push(link_str);
    }

    let entry_count = if data.len() >= cursor + 4 {
        u32::from_le_bytes([
            data[cursor],
            data[cursor + 1],
            data[cursor + 2],
            data[cursor + 3],
        ])
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

/// Parse a PROP binary completely.
///
/// Every length is checked against the buffer, a version below 2 is refused, and trailing bytes
/// after the last object are an error — a file that does not end where its own table says it
/// should is not one this parser understands.
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

/// Serialize a PROP binary (no `PTCH` prefix), the inverse of [`parse_prop_file`].
///
/// Fails only when a value does not fit its on-disk width (a link over 65 535 bytes, an object
/// body near 4 GiB) — never silently truncates.
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

        // Prepend 12-byte PTCH header
        let mut ptch_data = Vec::new();
        ptch_data.extend_from_slice(PTCH_SIGNATURE);
        ptch_data.extend_from_slice(&1u32.to_le_bytes()); // unk1
        ptch_data.extend_from_slice(&2u32.to_le_bytes()); // unk2
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
        // The header parser used elsewhere sees the same links and count.
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
        let data = b"PROP\x03\x00\x00\x00\x01\x00\x00\x00\x10\x00".to_vec(); // link length 16, but no payload
        let err = parse_prop_links(&data).unwrap_err();
        assert!(matches!(err, WadError::InvalidProp(_)));
    }
}
