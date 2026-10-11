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
