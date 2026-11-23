use dekan_classic::loading_name::rst;

struct XorShift64(u64);

impl XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next_u64() & 0xFF) as u8).collect()
    }
}

fn build_rst_v5(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut table = Vec::new();
    table.extend_from_slice(b"RST");
    table.push(5);
    table.extend_from_slice(&0u32.to_le_bytes());
    let updates: Vec<(String, String)> = entries
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    rst::with_texts(&table, &updates).expect("valid rst")
}

#[test]
fn property_rst_v5_roundtrip_preserves_multilingual_texts() {
    let initial = [
        ("game_character_displayname_zyra", "Zyra"),
        (
            "game_character_skin_displayname_zyra_36",
            "Spirit Blossom Zyra",
        ),
        ("game_character_skin_displayname_ahri_85", "阿狸 灵魂莲华"),
        ("game_character_skin_displayname_zed_1", "칼날폭풍 제드"),
        ("game_character_skin_displayname_lux_7", "Elementalist Lux"),
    ];
    let table = build_rst_v5(&initial);

    for (key, expected) in &initial {
        assert_eq!(
            rst::read_text(&table, rst::key_hash(key)).as_deref(),
            Some(*expected)
        );
    }

    let patched = rst::with_texts(
        &table,
        &[
            (
                "game_character_displayname_zyra".into(),
                "Zyra Florescer Espiritual".into(),
            ),
            ("bllt_character_displayname_zyra".into(), "Zyra".into()),
        ],
    )
    .expect("patch ok");

    assert_eq!(
        rst::read_text(&patched, rst::key_hash("game_character_displayname_zyra")).as_deref(),
        Some("Zyra Florescer Espiritual")
    );
    assert_eq!(
        rst::read_text(&patched, rst::key_hash("bllt_character_displayname_zyra")).as_deref(),
        Some("Zyra")
    );
    assert_eq!(
        rst::read_text(
            &patched,
            rst::key_hash("game_character_skin_displayname_ahri_85")
        )
        .as_deref(),
        Some("阿狸 灵魂莲华")
    );
}

#[test]
fn fuzz_malformed_rst_and_character_records_never_panic() {
    let mut rng = XorShift64(0x1234_5678_9ABC_DEF0);
    let base_table = build_rst_v5(&[("game_character_displayname_zed", "Zed")]);

    for iter in 0..1500u32 {
        let buf = if iter % 3 == 0 {
            let mut copy = base_table.clone();
            let idx = (rng.next_u64() as usize) % copy.len().max(1);
            copy[idx] ^= 0xFF;
            if iter % 6 == 0 && copy.len() > 1 {
                copy.truncate((rng.next_u64() as usize) % copy.len());
            }
            copy
        } else {
            let len = (rng.next_u64() % 256) as usize;
            rng.next_bytes(len)
        };

        let _ = rst::read_text(&buf, rng.next_u64() & rst::RST_MASK); // ignore-ok: fuzz test asserts no panic
        let _ = rst::with_texts(&buf, &[("k".into(), "X".into())]); // ignore-ok: fuzz test asserts no panic
        let _ = rst::patch_record_key(&buf, "Zed"); // ignore-ok: fuzz test asserts no panic
    }
}
