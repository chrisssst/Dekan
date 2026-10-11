use std::path::{Path, PathBuf};
use std::sync::Arc;

use dekan_wad::hash::{content_checksum, prop_key_hash, wad_path_hash};
use dekan_wad::prop::{PropEntry, PropFile, serialize_prop_file};
use dekan_wad::writer::{Payload, WadWriter, WriterEntry};

mod game;

pub use game::*;

pub const ZED: u32 = 238;
pub const SKIN_DATA: u32 = 0x9b67_e9f6;
pub const RESOURCES: u32 = 0xef3a_0f33;
pub const ANIMATION_GRAPH: u32 = 0xf5fb_07c7;
pub const CHAMPION_DATA: u32 = 0x45cd_899f;
pub const EMBED_ANIMATION: u32 = 0x1234_5678;
pub const FIELD_U32: u8 = 7;
pub const FIELD_STRING: u8 = 16;
pub const FIELD_EMBED: u8 = 0x83;
pub const FIELD_LINK: u8 = 0x84;

pub const CHAMPIONS: &str = "DATA/FINAL/Champions/Zed.wad.client";
pub const MAP11: &str = "DATA/FINAL/Maps/Shipping/Map11.wad.client";
pub const MAP22: &str = "DATA/FINAL/Maps/Shipping/Map22.wad.client";
pub const TEXTURE: &str = "assets/characters/zed/skins/skin69/zed_skin69_tx_cm.dds";
pub const NEW_TEXTURE: &str = "assets/characters/zed/skins/skin69/custom_glow.dds";

pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dekan_skin_pipeline_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&path); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&path).expect("scratch dir");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0); // ignore-ok: fixture cleanup
    }
}

pub fn field(body: &mut Vec<u8>, name: &str, kind: u8, value: &[u8]) {
    body.extend_from_slice(&prop_key_hash(name).to_le_bytes());
    body.push(kind);
    body.extend_from_slice(value);
}

pub fn string(value: &str) -> Vec<u8> {
    let mut out = u16::try_from(value.len())
        .expect("short string")
        .to_le_bytes()
        .to_vec();
    out.extend_from_slice(value.as_bytes());
    out
}

pub fn skin_object(
    character: &str,
    classification: Option<u32>,
    graph_skin: u32,
    mesh: &str,
) -> Vec<u8> {
    let mut graph = 1u16.to_le_bytes().to_vec();
    field(
        &mut graph,
        "animationGraphData",
        FIELD_LINK,
        &prop_key_hash(&format!(
            "Characters/{character}/Animations/Skin{graph_skin}"
        ))
        .to_le_bytes(),
    );
    let mut embed = EMBED_ANIMATION.to_le_bytes().to_vec();
    embed.extend_from_slice(&u32::try_from(graph.len()).expect("size").to_le_bytes());
    embed.extend_from_slice(&graph);

    let count = if classification.is_some() { 3u16 } else { 2 };
    let mut body = count.to_le_bytes().to_vec();
    if let Some(value) = classification {
        field(
            &mut body,
            "skinClassification",
            FIELD_U32,
            &value.to_le_bytes(),
        );
    }
    field(&mut body, "skinAnimationProperties", FIELD_EMBED, &embed);
    field(&mut body, "simpleSkin", FIELD_STRING, &string(mesh));
    body
}

pub fn prop(links: &[String], entries: Vec<PropEntry>) -> Vec<u8> {
    serialize_prop_file(&PropFile {
        version: 3,
        links: links.to_vec(),
        entries,
    })
    .expect("prop")
}

pub fn skin_bin(
    character: &str,
    skin: u32,
    classification: Option<u32>,
    graph_skin: u32,
) -> Vec<u8> {
    let key = format!("Characters/{character}/Skins/Skin{skin}");
    let mesh =
        format!("ASSETS/Characters/{character}/Skins/Skin{graph_skin}/{character}_Skin{skin}.skn");
    let mut links = vec![format!("DATA/Characters/{character}/{character}.bin")];
    if graph_skin != 0 {
        links.push(format!(
            "DATA/Characters/{character}/Animations/Skin{graph_skin}.bin"
        ));
    }
    prop(
        &links,
        vec![
            PropEntry {
                class_hash: SKIN_DATA,
                key_hash: prop_key_hash(&key),
                body: skin_object(character, classification, graph_skin, &mesh),
            },
            PropEntry {
                class_hash: RESOURCES,
                key_hash: prop_key_hash(&format!("{key}/Resources")),
                body: 0u16.to_le_bytes().to_vec(),
            },
        ],
    )
}

pub fn graph_bin(character: &str, skin: u32) -> Vec<u8> {
    prop(
        &[],
        vec![PropEntry {
            class_hash: ANIMATION_GRAPH,
            key_hash: prop_key_hash(&format!("Characters/{character}/Animations/Skin{skin}")),
            body: 0u16.to_le_bytes().to_vec(),
        }],
    )
}

pub fn character_bin(character: &str, companion: Option<&str>) -> Vec<u8> {
    let mut body = 1u16.to_le_bytes().to_vec();
    let mention = companion.map_or_else(String::new, |c| format!("Characters/{c}/Skins/Skin0"));
    field(&mut body, "companion", FIELD_STRING, &string(&mention));
    prop(
        &[],
        vec![PropEntry {
            class_hash: CHAMPION_DATA,
            key_hash: prop_key_hash(&format!("Characters/{character}")),
            body,
        }],
    )
}

pub fn raw(bytes: &[u8]) -> WriterEntry {
    WriterEntry {
        kind: 0,
        subchunk_count: 0,
        first_subchunk: 0,
        uncompressed_size: bytes.len() as u64,
        checksum: content_checksum(bytes),
        payload: Payload::Memory(Arc::from(bytes.to_vec())),
    }
}

pub fn write_wad(path: &Path, signature: u8, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = WadWriter::new([signature; 256]);
    for (name, bytes) in files {
        writer.insert(wad_path_hash(name), raw(bytes));
    }
    let mut bytes = writer.to_bytes().expect("wad");
    bytes[260..268].copy_from_slice(
        &u64::from(signature)
            .wrapping_mul(0x0101_0101_0101)
            .to_le_bytes(),
    );
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, &bytes).expect("write wad");
    bytes
}
