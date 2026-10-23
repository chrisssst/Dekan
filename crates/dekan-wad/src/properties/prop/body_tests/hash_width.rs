use super::*;

const WIDE_NAME: u64 = 0x0102_0304_0506_0708;
const WIDE_ITEM: u64 = 0x1111_2222_3333_4444;
const NAME_ENDING_IN_B5: u32 = 0x1234_56b5;

fn wide_field(body: &mut Vec<u8>, name: u32, value: u64, offsets: &mut Vec<usize>) {
    offsets.push(body.len() + 5);
    field(body, name, FIELD_HASH, &value.to_le_bytes());
}

fn body_with_wide_hashes() -> (Vec<u8>, Vec<usize>) {
    let mut offsets = Vec::new();
    let mut body = 5u16.to_le_bytes().to_vec();
    wide_field(&mut body, 0xA1, WIDE_NAME, &mut offsets);
    field(&mut body, NAME_ENDING_IN_B5, FIELD_HASH, &OLD.to_le_bytes());

    let mut inner = 2u16.to_le_bytes().to_vec();
    let embed_body_at = body.len() + 5 + 8;
    let mut inner_offsets = Vec::new();
    wide_field(&mut inner, 0x77, WIDE_NAME, &mut inner_offsets);
    field(&mut inner, 0x5555_55b5, FIELD_LINK, &OLD.to_le_bytes());
    let mut embed = 0xC1u32.to_le_bytes().to_vec();
    embed.extend_from_slice(&u32::try_from(inner.len()).expect("small").to_le_bytes());
    embed.extend_from_slice(&inner);
    field(&mut body, 0xA3, FIELD_EMBED, &embed);
    offsets.extend(inner_offsets.iter().map(|at| embed_body_at + at));

    let items_at = body.len() + 5 + 1 + 4 + 4;
    let mut list = vec![FIELD_HASH];
    list.extend_from_slice(&20u32.to_le_bytes());
    list.extend_from_slice(&2u32.to_le_bytes());
    list.extend_from_slice(&WIDE_ITEM.to_le_bytes());
    list.extend_from_slice(&WIDE_ITEM.to_le_bytes());
    field(&mut body, 0xA4, FIELD_LIST, &list);
    offsets.extend([items_at, items_at + 8]);

    let key_at = body.len() + 5 + 2 + 4 + 4;
    let mut map = vec![FIELD_HASH, FIELD_U32];
    map.extend_from_slice(&16u32.to_le_bytes());
    map.extend_from_slice(&1u32.to_le_bytes());
    map.extend_from_slice(&WIDE_ITEM.to_le_bytes());
    map.extend_from_slice(&7u32.to_le_bytes());
    field(&mut body, 0xA5, FIELD_MAP, &map);
    offsets.push(key_at);

    offsets.sort_unstable();
    (body, offsets)
}

#[test]
fn a_body_with_eight_byte_hashes_is_read_and_written_back_byte_for_byte() {
    let (body, offsets) = body_with_wide_hashes();
    assert_eq!(tree::hash_widths(&body).expect("widths"), offsets);
    let fields = tree::parse_fields(&body).expect("parse");
    assert_eq!(tree::write_fields(&fields).expect("write"), body);
    let name = field_value(&body, &[0xA1]).expect("walk").expect("name");
    assert_eq!(name.bytes, WIDE_NAME.to_le_bytes());
    assert_eq!(name.as_u32(), None, "a wide hash is not a u32");
    let nested = field_value(&body, &[0xA3, 0x5555_55b5])
        .expect("walk")
        .expect("link");
    assert_eq!(nested.as_u32(), Some(OLD));
    assert!(!flatten_fields(&body).expect("flatten").is_empty());
}

#[test]
fn only_four_byte_references_move_next_to_eight_byte_hashes() {
    let (mut body, _) = body_with_wide_hashes();
    let original = body.clone();
    assert_eq!(reference_values(&body).expect("references"), vec![OLD, OLD]);
    let map = std::collections::BTreeMap::from([(OLD, NEW)]);
    assert_eq!(remap_references(&mut body, &map).expect("remap"), 2);
    assert_eq!(body.len(), original.len());
    let name = field_value(&body, &[0xA1]).expect("walk").expect("name");
    assert_eq!(name.bytes, WIDE_NAME.to_le_bytes());
}

#[test]
fn a_body_with_only_four_byte_hashes_keeps_an_empty_record() {
    let body = body_with_references_in_every_container();
    assert!(tree::hash_widths(&body).expect("widths").is_empty());
}

#[test]
fn a_map_whose_hash_widths_cannot_be_told_apart_is_refused() {
    let mut body = 1u16.to_le_bytes().to_vec();
    let mut map = vec![FIELD_HASH, FIELD_HASH];
    map.extend_from_slice(&16u32.to_le_bytes());
    map.extend_from_slice(&1u32.to_le_bytes());
    map.extend_from_slice(&[0xAB; 12]);
    field(&mut body, 1, FIELD_MAP, &map);
    assert!(tree::hash_widths(&body).is_err());
    assert!(tree::parse_fields(&body).is_err());
}

#[test]
fn every_truncation_of_a_wide_body_is_an_error_or_written_back_unchanged() {
    let (body, _) = body_with_wide_hashes();
    for len in 0..body.len() {
        let prefix = &body[..len];
        if let Ok(fields) = tree::parse_fields(prefix) {
            assert_eq!(
                tree::write_fields(&fields).expect("write"),
                prefix,
                "length {len}"
            );
        }
    }
}
