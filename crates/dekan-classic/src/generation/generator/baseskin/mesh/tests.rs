use super::*;

fn sample_modern_skl() -> Vec<u8> {
    let joints_off = 64usize;
    let table_off = joints_off + SKL_JOINT_SIZE;
    let names_off = table_off + SKL_TABLE_ENTRY_SIZE;
    let name = b"Root\0";
    let mut data = vec![0u8; names_off + name.len()];
    while data.len() % 4 != 0 {
        data.push(0);
    }
    let size = data.len() as u32;
    data[0..4].copy_from_slice(&size.to_le_bytes());
    data[4..8].copy_from_slice(&SKL_FORMAT_TOKEN.to_le_bytes());
    data[SKL_VERSION..SKL_VERSION + 4].copy_from_slice(&0u32.to_le_bytes());
    data[SKL_JOINT_COUNT..SKL_JOINT_COUNT + 2].copy_from_slice(&1u16.to_le_bytes());
    data[SKL_SECTION_OFFSETS..SKL_SECTION_OFFSETS + 4]
        .copy_from_slice(&(joints_off as i32).to_le_bytes());
    data[SKL_SECTION_OFFSETS + 4..SKL_SECTION_OFFSETS + 8]
        .copy_from_slice(&(table_off as i32).to_le_bytes());
    data[SKL_SECTION_OFFSETS + 20..SKL_SECTION_OFFSETS + 24]
        .copy_from_slice(&(names_off as i32).to_le_bytes());

    let root_hash = elf_hash_lower(b"Root");
    let field_at = joints_off + JOINT_NAME_OFFSET;
    let rel = (names_off as i32) - (field_at as i32);
    let joint = build_null_joint(0, root_hash, rel);
    data[joints_off..joints_off + SKL_JOINT_SIZE].copy_from_slice(&joint);

    data[table_off..table_off + 2].copy_from_slice(&0i16.to_le_bytes());
    data[table_off + 4..table_off + 8].copy_from_slice(&root_hash.to_le_bytes());
    data[names_off..names_off + name.len()].copy_from_slice(name);
    data
}

fn sample_skn_v4() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&SKN_MAGIC.to_le_bytes());
    data.extend_from_slice(&4u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes());
    let mut range = [0u8; SKN_RANGE_SIZE];
    range[0..5].copy_from_slice(b"Body\0");
    range[RANGE_VERTEX_START..RANGE_VERTEX_START + 4].copy_from_slice(&0u32.to_le_bytes());
    range[RANGE_VERTEX_COUNT..RANGE_VERTEX_COUNT + 4].copy_from_slice(&3u32.to_le_bytes());
    range[72..76].copy_from_slice(&0u32.to_le_bytes());
    range[76..80].copy_from_slice(&3u32.to_le_bytes());
    data.extend_from_slice(&range);
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&3u32.to_le_bytes());
    data.extend_from_slice(&3u32.to_le_bytes());
    data.extend_from_slice(&52u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&[0u8; SKN_BOUNDS_SIZE]);
    for idx in [0u16, 1u16, 2u16] {
        data.extend_from_slice(&idx.to_le_bytes());
    }
    for v in 0u8..3 {
        data.extend_from_slice(&[v.wrapping_add(1); 52]);
    }
    data
}

#[test]
fn rebuild_modern_skl_adds_null_bone_and_updates_table() {
    let orig = sample_modern_skl();
    let (rebuilt, kind) = rebuild_mesh(orig.clone());
    assert_eq!(kind, "skl_null_joint");
    assert_eq!(u32_le(&rebuilt, 0), Some(rebuilt.len() as u32));
    assert_eq!(u16_le(&rebuilt, SKL_JOINT_COUNT), Some(2));
    assert_ne!(&rebuilt[..orig.len()], &orig[..]);

    let joints_off = i32_le(&rebuilt, SKL_SECTION_OFFSETS).expect("joints") as usize;
    let table_off = i32_le(&rebuilt, SKL_SECTION_OFFSETS + 4).expect("table") as usize;
    let root_field = joints_off + JOINT_NAME_OFFSET;
    let root_name_at = (root_field as i32 + i32_le(&rebuilt, root_field).expect("rel")) as usize;
    assert_eq!(&rebuilt[root_name_at..root_name_at + 5], b"Root\0");

    let null_field = joints_off + SKL_JOINT_SIZE + JOINT_NAME_OFFSET;
    let null_name_at = (null_field as i32 + i32_le(&rebuilt, null_field).expect("rel")) as usize;
    assert_eq!(&rebuilt[null_name_at..null_name_at + 10], b"null_bone\0");

    let h0 = u32_le(&rebuilt, table_off + 4).expect("h0");
    let h1 = u32_le(&rebuilt, table_off + SKL_TABLE_ENTRY_SIZE + 4).expect("h1");
    assert!(h0 <= h1);
    let null_hash = elf_hash_lower(NULL_JOINT_NAME);
    assert!(h0 == null_hash || h1 == null_hash);

    let (rebuilt_again, _) = rebuild_mesh(rebuilt.clone());
    assert_eq!(rebuilt_again, rebuilt);
}

#[test]
fn rebuild_skn_v4_duplicates_last_vertex_and_updates_counts() {
    let orig = sample_skn_v4();
    let (rebuilt, kind) = rebuild_mesh(orig.clone());
    assert_eq!(kind, "skn_null_vertex");
    assert_eq!(rebuilt.len(), orig.len() + 52);
    assert_ne!(&rebuilt[..orig.len()], &orig[..]);
    assert_eq!(u32_le(&rebuilt, 12 + RANGE_VERTEX_COUNT), Some(4));
    assert_eq!(u32_le(&rebuilt, 12 + SKN_RANGE_SIZE + 4 + 4), Some(4));
    assert_eq!(&rebuilt[rebuilt.len() - 52..], &orig[orig.len() - 52..]);
}
