use std::collections::BTreeSet;
use std::path::Path;

use dekan_classic::forms::{gear_keys, skn_submesh_names};
use dekan_classic::gear_toggle::gear_swap;
use dekan_classic::generator::StandardChampion;
use dekan_classic::loading_name::rst;
use dekan_wad::hash::{prop_key_hash as h, wad_path_hash};
use dekan_wad::prop::parse_prop_file;
use dekan_wad::prop::tree::{self, Value};

const SKIN_DATA_CLASS: u32 = 0x9b67_e9f6;
const VFX_SYSTEM_CLASS: u32 = 0x45cd_8998;
const STATIC_MATERIAL_CLASS: u32 = 0xff9d_3409;

fn split_submesh_list(raw: &str) -> BTreeSet<u32> {
    raw.split(|c: char| c.is_whitespace() || matches!(c, ',' | '|' | ':'))
        .filter(|s| !s.is_empty())
        .map(h)
        .collect()
}

fn raw_text(value: &Value) -> Option<String> {
    match value {
        Value::Raw { kind: 16, bytes } if bytes.len() >= 2 => {
            Some(String::from_utf8_lossy(&bytes[2..]).into_owned())
        }
        _ => None,
    }
}

fn hashes_of(fields: &[tree::Field], name: &str) -> Vec<u32> {
    tree::field(fields, h(name))
        .and_then(Value::items)
        .map(|items| items.iter().filter_map(Value::as_u32).collect())
        .unwrap_or_default()
}

fn valid_texture_bytes(bytes: &[u8]) -> bool {
    (bytes.len() >= 12
        && bytes.starts_with(b"TEX\0")
        && u16::from_le_bytes([bytes[4], bytes[5]]) > 0
        && u16::from_le_bytes([bytes[6], bytes[7]]) > 0)
        || (bytes.len() >= 128 && bytes.starts_with(b"DDS "))
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xFF\xD8\xFF")
        || (bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
}

fn has_skin_numbered_ref(value: &str, character: &str) -> bool {
    for segment in value.split('_') {
        let low = segment.to_ascii_lowercase();
        if let Some(at) = low.rfind("skin") {
            let after = &low[at + 4..];
            let digit_len = after.bytes().take_while(u8::is_ascii_digit).count();
            let (digits, tail, head) = (&after[..digit_len], &after[digit_len..], &low[..at]);
            if !digits.is_empty()
                && digits.bytes().any(|b| b != b'0')
                && head.bytes().all(|b| b.is_ascii_alphabetic())
                && tail.bytes().all(|b| b.is_ascii_alphabetic())
            {
                return true;
            }
        }
    }
    let lower = value.to_ascii_lowercase();
    let needle = format!("{}skin", character.to_ascii_lowercase());
    let mut from = 0;
    while let Some(pos) = lower[from..].find(&needle) {
        let after = from + pos + needle.len();
        if lower
            .as_bytes()
            .get(after)
            .is_some_and(|b| b.is_ascii_digit() && *b != b'0')
        {
            return true;
        }
        from = after;
    }
    false
}

fn verify_audio_and_deep_entries(
    champion: &StandardChampion,
    main: &str,
    wad_root: &Path,
    gen_bytes: &[u8],
    gen_fields: &[tree::Field],
) {
    let gen_prop = parse_prop_file(gen_bytes).expect("parse gen prop");
    let mut defined_keys: BTreeSet<u32> = gen_prop.entries.iter().map(|e| e.key_hash).collect();
    for link in &gen_prop.links {
        let lower = link.to_ascii_lowercase();
        if let Some(b) = std::fs::read(wad_root.join(&lower))
            .ok()
            .or_else(|| champion.read_path(&lower).ok().flatten())
            && let Ok(lp) = parse_prop_file(&b)
        {
            defined_keys.extend(lp.entries.into_iter().map(|e| e.key_hash));
        }
    }
    for entry in &gen_prop.entries {
        let ef = tree::parse_fields(&entry.body).expect("parse entry fields");
        if entry.class_hash == VFX_SYSTEM_CLASS {
            assert!(!ef.is_empty());
        } else if entry.class_hash == STATIC_MATERIAL_CLASS
            && let Some(samplers) = tree::field(&ef, h("samplerValues")).and_then(Value::items)
        {
            for s in samplers.iter().filter_map(Value::fields) {
                if let Some(tex) = tree::field(s, h("textureName"))
                    .or_else(|| tree::field(s, h("texturePath")))
                    .and_then(raw_text)
                {
                    let lower = tex.to_ascii_lowercase();
                    if let Some(b) = std::fs::read(wad_root.join(&lower))
                        .ok()
                        .or_else(|| champion.read_path(&lower).ok().flatten())
                    {
                        assert!(valid_texture_bytes(&b), "invalid sampler texture {lower}");
                    }
                }
            }
        }
    }
    if let Some(resolver) = tree::field(gen_fields, h("mResourceResolver")).and_then(Value::as_u32)
        && resolver != 0
    {
        assert!(defined_keys.contains(&resolver));
    }
    if let Some(cname) = tree::field(gen_fields, h("championSkinName")).and_then(raw_text) {
        assert!(!has_skin_numbered_ref(&cname, main), "stale name {cname}");
    }
    if let Some(audio) = tree::field(gen_fields, h("skinAudioProperties")).and_then(Value::fields) {
        if let Some(tags) = tree::field(audio, h("tagEventList")).and_then(Value::items) {
            for t in tags.iter().filter_map(raw_text) {
                assert!(!has_skin_numbered_ref(&t, main), "stale audio tag {t}");
            }
        }
        if let Some(units) = tree::field(audio, h("bankUnits")).and_then(Value::items) {
            for uf in units.iter().filter_map(Value::fields) {
                if let Some(uname) = tree::field(uf, h("name")).and_then(raw_text) {
                    assert!(!has_skin_numbered_ref(&uname, main), "stale unit {uname}");
                }
                if let Some(paths) = tree::field(uf, h("bankPath")).and_then(Value::items) {
                    for p in paths.iter().filter_map(raw_text) {
                        let lower = p.to_ascii_lowercase();
                        let local = wad_root.join(&lower);
                        if local.is_file() && lower.ends_with(".bnk") {
                            let b = std::fs::read(&local).expect("read bnk");
                            assert!(dekan_classic::sound_bank::parse(&b).is_ok());
                        }
                    }
                }
            }
        }
    }
}

fn verify_generated_exalted_or_form_skin(game_dir: &Path, alias: &str, skin: u32) {
    let dir = std::env::temp_dir().join(format!(
        "dekan_12domain_{}_{}_{}",
        alias.to_ascii_lowercase(),
        skin,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: pre-clean test scratch dir
    std::fs::create_dir_all(&dir).expect("create scratch dir");

    let champion = StandardChampion::open(game_dir, alias).expect("open champion");
    let main = alias.to_ascii_lowercase();
    let folder = champion
        .build_mod(skin, None, &dir)
        .expect("build_mod succeeds");
    let mod_root = dir.join(&folder);
    let wad_root = mod_root.join("WAD").join(format!("{alias}.wad.client"));

    for meta in ["META/info.json", "META/manifest.json"] {
        let bytes = std::fs::read(mod_root.join(meta)).expect("read meta");
        serde_json::from_slice::<serde_json::Value>(&bytes).expect("valid meta json");
    }

    let src_bytes = champion
        .read_skin_bin(&main, skin)
        .expect("read source skin")
        .expect("source skin present");
    let gen_bytes = std::fs::read(
        wad_root
            .join("data/characters")
            .join(&main)
            .join("skins/skin0.bin"),
    )
    .expect("read generated skin0.bin");

    let src_prop = parse_prop_file(&src_bytes).expect("parse source prop");
    let gen_prop = parse_prop_file(&gen_bytes).expect("parse generated prop");
    for link in &src_prop.links {
        assert!(
            gen_prop.links.contains(link),
            "{alias} skin{skin} dropped link {link}"
        );
    }

    let src_skin = src_prop
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
        .expect("src skin entry");
    let gen_skin = gen_prop
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA_CLASS)
        .expect("gen skin entry");
    let src_fields = tree::parse_fields(&src_skin.body).expect("src fields");
    let gen_fields = tree::parse_fields(&gen_skin.body).expect("gen fields");
    verify_audio_and_deep_entries(&champion, &main, &wad_root, &gen_bytes, &gen_fields);

    let graph_of = |f: &[tree::Field]| {
        tree::field(f, h("skinAnimationProperties"))
            .and_then(Value::fields)
            .and_then(|a| tree::field(a, h("animationGraphData")))
            .and_then(Value::as_u32)
    };
    assert_eq!(graph_of(&src_fields), graph_of(&gen_fields));

    let gen_mesh = tree::field(&gen_fields, h("skinMeshProperties"))
        .and_then(Value::fields)
        .expect("gen skinMeshProperties");
    let skn_rel = tree::field(gen_mesh, h("simpleSkin"))
        .and_then(raw_text)
        .expect("simpleSkin")
        .to_ascii_lowercase();
    let skl_rel = tree::field(gen_mesh, h("skeleton"))
        .and_then(raw_text)
        .expect("skeleton")
        .to_ascii_lowercase();
    assert_ne!(wad_path_hash(&skn_rel), 0);
    assert_ne!(wad_path_hash(&skl_rel), 0);

    let skn_bytes = std::fs::read(wad_root.join(&skn_rel))
        .ok()
        .or_else(|| champion.read_path(&skn_rel).ok().flatten())
        .expect("skn bytes");
    let parsed_skn = dekan_classic::skinned_mesh::parse(&skn_bytes).expect("parse skn");
    parsed_skn
        .validate_submesh_indices()
        .expect("valid submesh indices");
    assert!(!parsed_skn.submeshes.is_empty() && (52..=72).contains(&parsed_skn.vertex_size));
    let skn_parts = skn_submesh_names(&skn_bytes).expect("skn submesh names");
    let skn_hashes: BTreeSet<u32> = skn_parts.iter().map(|n| h(n)).collect();

    let skl_bytes = std::fs::read(wad_root.join(&skl_rel))
        .ok()
        .or_else(|| champion.read_path(&skl_rel).ok().flatten())
        .expect("skl bytes");
    let parsed_skl = dekan_classic::skeleton::parse(&skl_bytes).expect("parse skl");
    assert!(!parsed_skl.joints.is_empty());

    if let Some(items) = tree::field(gen_mesh, h("materialOverride")).and_then(Value::items) {
        for item in items {
            if let Some(sub) = item
                .fields()
                .and_then(|f| tree::field(f, h("submesh")))
                .and_then(raw_text)
                && sub
                    .rsplit_once("_F")
                    .is_some_and(|(_, t)| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
            {
                assert!(skn_hashes.contains(&h(&sub)), "missing part {sub}");
            }
        }
    }

    let src_mesh = tree::field(&src_fields, h("skinMeshProperties"))
        .and_then(Value::fields)
        .expect("src skinMeshProperties");
    let src_skn_rel = tree::field(src_mesh, h("simpleSkin"))
        .and_then(raw_text)
        .expect("src simpleSkin")
        .to_ascii_lowercase();
    let src_skn_bytes = champion
        .read_path(&src_skn_rel)
        .expect("read src skn")
        .expect("src skn present");
    let src_skn_parts = skn_submesh_names(&src_skn_bytes).expect("src skn names");
    let src_init_hide = tree::field(src_mesh, h("initialSubmeshToHide"))
        .and_then(raw_text)
        .map(|s| split_submesh_list(&s))
        .unwrap_or_default();

    let keys = gear_keys(&src_bytes).expect("gear keys");
    if keys.len() >= 2 && champion.gear_count(0) == 0 {
        let gear0_body = champion.gear_body(&src_bytes, keys[0]).expect("gear 0");
        let gear0_hide: BTreeSet<u32> = gear_swap(&gear0_body)
            .expect("gear 0 swap")
            .hide
            .into_iter()
            .collect();
        let gen_init_hide = tree::field(gen_mesh, h("initialSubmeshToHide"))
            .and_then(raw_text)
            .map(|s| split_submesh_list(&s))
            .unwrap_or_default();

        let mut src_cond_hide = BTreeSet::new();
        if let Some(conds) =
            tree::field(&src_fields, h("PersistentEffectConditions")).and_then(Value::items)
        {
            for cf in conds.iter().filter_map(Value::fields) {
                src_cond_hide.extend(hashes_of(cf, "SubmeshesToHide"));
            }
        }

        let mut cond_hide = BTreeSet::new();
        if let Some(conds) =
            tree::field(&gen_fields, h("PersistentEffectConditions")).and_then(Value::items)
        {
            for cf in conds.iter().filter_map(Value::fields) {
                if tree::field(cf, h("OwnerCondition")).and_then(Value::class)
                    == Some(h("NotMaterialDriver"))
                {
                    cond_hide.extend(hashes_of(cf, "SubmeshesToHide"));
                }
            }
        }

        for part in &src_skn_parts {
            let ph = h(part);
            if !src_init_hide.contains(&ph)
                && !gear0_hide.contains(&ph)
                && !src_cond_hide.contains(&ph)
            {
                assert!(!gen_init_hide.contains(&ph));
                assert!(!cond_hide.contains(&ph));
            }
        }
    }

    for form in 0..champion.gear_count(skin) as u32 {
        let f_folder = champion
            .build_mod_form(skin, None, form, &dir)
            .expect("build_mod_form");
        let f_wad = dir
            .join(&f_folder)
            .join("WAD")
            .join(format!("{alias}.wad.client"));
        let f_bin = std::fs::read(
            f_wad
                .join("data/characters")
                .join(&main)
                .join("skins/skin0.bin"),
        )
        .expect("read form skin0.bin");
        let f_prop = parse_prop_file(&f_bin).expect("parse form prop");
        let f_skin = f_prop
            .entries
            .iter()
            .find(|e| e.class_hash == SKIN_DATA_CLASS)
            .expect("form skin entry");
        let f_fields = tree::parse_fields(&f_skin.body).expect("form fields");
        verify_audio_and_deep_entries(&champion, &main, &f_wad, &f_bin, &f_fields);
    }

    let record_rel = format!("data/characters/{main}/{main}.bin");
    if let Ok(Some(rec)) = champion.read_path(&record_rel)
        && let Some(patched) = rst::patch_record_key(&rec, alias)
    {
        assert!(parse_prop_file(&patched).is_ok());
    }

    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: cleanup test scratch dir
}

#[test]
fn real_game_exalted_and_form_skins_pass_all_12_domain_validations() {
    let candidates = [
        Path::new(r"D:\Riot Games\League of Legends\Game"),
        Path::new(r"C:\Riot Games\League of Legends\Game"),
    ];
    let Some(game_dir) = candidates.into_iter().find(|p| {
        p.join("DATA/FINAL/Champions/Mordekaiser.wad.client")
            .is_file()
    }) else {
        return;
    };

    for (alias, skin) in [
        ("Mordekaiser", 54),
        ("Sett", 66),
        ("Diana", 77),
        ("Lux", 7),
        ("Sona", 6),
        ("Morgana", 80),
        ("Jinx", 60),
        ("MissFortune", 16),
        ("Viego", 43),
        ("Ahri", 85),
        ("Kaisa", 71),
    ] {
        verify_generated_exalted_or_form_skin(game_dir, alias, skin);
    }
}
