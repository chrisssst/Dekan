use super::*;

pub(crate) fn joint(name: &str, parent: i16, translation: [f32; 3]) -> Joint {
    Joint {
        flags: 0,
        id: 0,
        parent,
        hash: joint_hash(name),
        radius: 2.1,
        local: Transform {
            translation,
            ..Transform::IDENTITY
        },
        inverse_bind: Transform {
            translation: translation.map(|v| -v),
            ..Transform::IDENTITY
        },
        name: name.to_owned(),
    }
}

pub(crate) fn skeleton(joints: Vec<Joint>, influences: Vec<u16>) -> Skeleton {
    let joints = joints
        .into_iter()
        .enumerate()
        .map(|(i, mut j)| {
            j.id = i16::try_from(i).expect("id");
            j
        })
        .collect();
    Skeleton {
        version: 0,
        flags: 0,
        name: String::new(),
        asset_name: String::new(),
        joints,
        influences,
    }
}

#[test]
fn joint_names_hash_like_the_game_does() {
    assert_eq!(joint_hash("Weapon"), 0x07db_875e);
    assert_eq!(joint_hash("weapon"), joint_hash("WEAPON"));
}

#[test]
fn a_written_skeleton_reads_back_whole_with_every_section_aligned() {
    let original = skeleton(
        vec![
            joint("Root", -1, [0.0, 0.0, 0.0]),
            joint("Pelvis", 0, [0.0, 90.5, 1.25]),
            joint("L_Hand_Buffbone_Long_Name", 1, [-3.0, 2.0, 0.5]),
        ],
        vec![1, 2],
    );
    let bytes = write(&original).expect("write");
    assert_eq!(bytes.len() % 4, 0);
    assert_eq!(
        u32::from_le_bytes(bytes[0..4].try_into().expect("size")) as usize,
        bytes.len()
    );
    assert_eq!(parse(&bytes).expect("parse"), original);
    assert_eq!(
        write(&parse(&bytes).expect("again")).expect("rewrite"),
        bytes
    );
}

#[test]
fn an_asset_name_equal_to_the_name_shares_its_string_like_the_game_files() {
    let read =
        |bytes: &[u8], at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("offset"));
    let mut shared = skeleton(vec![joint("Root", -1, [0.0; 3])], Vec::new());
    shared.name = "abc".into();
    shared.asset_name = "abc".into();
    let bytes = write(&shared).expect("write");
    assert_eq!(read(&bytes, 32), read(&bytes, 36));
    assert_eq!(parse(&bytes).expect("parse"), shared);
    let empty = write(&skeleton(vec![joint("Root", -1, [0.0; 3])], Vec::new())).expect("write");
    assert_eq!(read(&empty, 32) + 4, read(&empty, 36));
    assert_eq!(empty.len(), bytes.len() + 4);
}

#[test]
fn the_hash_index_is_sorted_so_the_game_can_search_it() {
    let bytes = write(&skeleton(
        vec![
            joint("Zeta", -1, [0.0; 3]),
            joint("Alpha", 0, [1.0; 3]),
            joint("Mid", 0, [2.0; 3]),
        ],
        Vec::new(),
    ))
    .expect("write");
    let at = HEADER + 3 * JOINT;
    let hashes: Vec<u32> = (0..3)
        .map(|i| {
            let o = at + i * INDEX_ENTRY + 4;
            u32::from_le_bytes(bytes[o..o + 4].try_into().expect("hash"))
        })
        .collect();
    let mut sorted = hashes.clone();
    sorted.sort_unstable();
    assert_eq!(hashes, sorted);
}

#[test]
fn damaged_skeletons_are_typed_errors_never_panics() {
    let bytes = write(&skeleton(
        vec![
            joint("Root", -1, [0.0; 3]),
            joint("Spine", 0, [0.0, 1.0, 0.0]),
        ],
        vec![0, 1],
    ))
    .expect("write");
    for cut in 0..bytes.len() {
        let _ = parse(&bytes[..cut]); // ignore-ok: only the absence of a panic is checked
    }
    let mut wrong_token = bytes.clone();
    wrong_token[4] ^= 0xFF;
    assert!(parse(&wrong_token).is_err());
    let mut bad_parent = bytes.clone();
    let parent_at = HEADER + JOINT + 4;
    bad_parent[parent_at..parent_at + 2].copy_from_slice(&7i16.to_le_bytes());
    assert!(parse(&bad_parent).is_err());
    let mut bad_influence = bytes;
    let influence_at = HEADER + 2 * JOINT + 2 * INDEX_ENTRY;
    bad_influence[influence_at..influence_at + 2].copy_from_slice(&9u16.to_le_bytes());
    assert!(parse(&bad_influence).is_err());
}

#[test]
fn transforms_match_within_tolerance_and_a_flipped_quaternion_is_the_same_rotation() {
    let a = Transform {
        translation: [1.0, 2.0, 3.0],
        scale: [1.0; 3],
        rotation: [
            std::f32::consts::FRAC_1_SQRT_2,
            -std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            0.0,
        ],
    };
    let flipped = Transform {
        rotation: [
            -std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            0.0,
        ],
        translation: [1.005, 2.0, 3.0],
        ..a
    };
    assert!(a.same_as(&flipped));
    let turned = Transform {
        rotation: [0.034, -0.042, 0.452, 0.890],
        ..a
    };
    let other = Transform {
        rotation: [0.037, -0.040, 0.393, 0.917],
        ..a
    };
    assert!(!turned.same_as(&other));
    let moved = Transform {
        translation: [1.5, 2.0, 3.0],
        ..a
    };
    assert!(!a.same_as(&moved));
}
