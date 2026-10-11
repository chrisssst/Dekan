use super::*;

pub(crate) fn object_bytes(kind: u8, id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![kind];
    out.extend_from_slice(
        &u32::try_from(payload.len() + 4)
            .expect("size")
            .to_le_bytes(),
    );
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub(crate) fn bank(objects: &[(u8, u32, Vec<u8>)]) -> Vec<u8> {
    let mut hierarchy = u32::try_from(objects.len())
        .expect("count")
        .to_le_bytes()
        .to_vec();
    for (kind, id, payload) in objects {
        hierarchy.extend(object_bytes(*kind, *id, payload));
    }
    let mut out = Vec::new();
    for (tag, data) in [
        (
            *b"BKHD",
            [145u32.to_le_bytes(), 7u32.to_le_bytes()].concat(),
        ),
        (*b"HIRC", hierarchy),
        (*b"STID", vec![1, 2, 3]),
    ] {
        out.extend_from_slice(&tag);
        out.extend_from_slice(&u32::try_from(data.len()).expect("len").to_le_bytes());
        out.extend_from_slice(&data);
    }
    out
}

#[test]
fn ids_are_the_lowercase_fnv1_hash_wwise_uses() {
    assert_eq!(wwise_id("gear"), 0x45e8_2e50);
    assert_eq!(wwise_id("Gear"), wwise_id("gear"));
}

#[test]
fn a_bank_reads_back_whole_and_rewrites_byte_for_byte() {
    let bytes = bank(&[
        (3, 10, vec![1, 25, 0, 0, 0, 0]),
        (4, 11, vec![1, 10, 0, 0, 0]),
    ]);
    let parsed = parse(&bytes).expect("parse");
    assert_eq!(parsed.version(), Some(145));
    assert_eq!(parsed.write().expect("write"), bytes);
    let objects = parsed.objects().expect("objects");
    assert_eq!(objects.len(), 2);
    assert_eq!((objects[1].kind, objects[1].id), (4, 11));
}

#[test]
fn appended_objects_land_at_the_end_of_the_hierarchy_and_the_rest_is_untouched() {
    let original = bank(&[(3, 10, vec![9; 6])]);
    let mut parsed = parse(&original).expect("parse");
    let added = Object {
        kind: 4,
        id: 99,
        payload: vec![1, 10, 0, 0, 0],
    };
    parsed.append(std::slice::from_ref(&added)).expect("append");
    let written = parsed.write().expect("write");
    assert_eq!(
        written,
        bank(&[(3, 10, vec![9; 6]), (4, 99, vec![1, 10, 0, 0, 0])])
    );
    let again = parse(&written).expect("again");
    assert_eq!(again.objects().expect("objects").last(), Some(&added));
    assert_eq!(
        &written[written.len() - 11..],
        &original[original.len() - 11..]
    );
}

#[test]
fn damaged_banks_are_typed_errors_never_panics() {
    let bytes = bank(&[(3, 10, vec![1, 25, 0, 0, 0, 0])]);
    for cut in 0..bytes.len() {
        let _ = parse(&bytes[..cut]); // ignore-ok: only the absence of a panic is checked
    }
    assert!(parse(b"RIFF....").is_err());
    let mut lying = bytes.clone();
    let count_at = 8 + 8 + 8;
    lying[count_at..count_at + 4].copy_from_slice(&5u32.to_le_bytes());
    assert!(parse(&lying).is_err(), "a count past the objects");
    let mut short = bytes;
    let size_at = 8 + 8 + 8 + 4 + 1;
    short[size_at..size_at + 4].copy_from_slice(&2u32.to_le_bytes());
    assert!(parse(&short).is_err(), "an object shorter than its id");
    let mut without = parse(&bank(&[])).expect("empty");
    without.sections.retain(|s| &s.tag != b"HIRC");
    assert!(without.append(&[]).is_err());
}
