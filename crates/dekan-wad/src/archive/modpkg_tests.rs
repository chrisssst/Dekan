use super::*;
use std::io::Cursor;

struct Chunk {
    path_hash: u64,
    content: Vec<u8>,
    zstd: bool,
    layer: u32,
    wad: u32,
}

fn chunk(path_hash: u64, content: &[u8], zstd: bool, layer: u32, wad: u32) -> Chunk {
    Chunk {
        path_hash,
        content: content.to_vec(),
        zstd,
        layer,
        wad,
    }
}

fn package(layers: &[&str], wads: &[&str], chunks: &[Chunk]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(chunks.len()).expect("count").to_le_bytes());
    out.extend_from_slice(b"sig");
    out.extend_from_slice(&u32::try_from(layers.len()).expect("count").to_le_bytes());
    for (priority, name) in layers.iter().enumerate() {
        out.extend_from_slice(&u32::try_from(name.len()).expect("len").to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&i32::try_from(priority).expect("priority").to_le_bytes());
    }
    out.extend_from_slice(&u32::try_from(chunks.len()).expect("count").to_le_bytes());
    for c in chunks {
        out.extend_from_slice(format!("{:016x}.bin", c.path_hash).as_bytes());
        out.push(0);
    }
    out.extend_from_slice(&u32::try_from(wads.len()).expect("count").to_le_bytes());
    for name in wads {
        out.extend_from_slice(name.as_bytes());
        out.push(0);
    }
    out.resize(out.len().next_multiple_of(8), 0);
    let stored: Vec<Vec<u8>> = chunks
        .iter()
        .map(|c| {
            if c.zstd {
                zstd::bulk::compress(&c.content, 3).expect("zstd")
            } else {
                c.content.clone()
            }
        })
        .collect();
    let mut data_offset = (out.len() + chunks.len() * 61) as u64;
    for (index, (c, bytes)) in chunks.iter().zip(&stored).enumerate() {
        out.extend_from_slice(&c.path_hash.to_le_bytes());
        out.extend_from_slice(&data_offset.to_le_bytes());
        out.push(u8::from(c.zstd));
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(&(c.content.len() as u64).to_le_bytes());
        out.extend_from_slice(&content_checksum(bytes).to_le_bytes());
        out.extend_from_slice(&content_checksum(&c.content).to_le_bytes());
        out.extend_from_slice(&u32::try_from(index).expect("index").to_le_bytes());
        out.extend_from_slice(&c.layer.to_le_bytes());
        out.extend_from_slice(&c.wad.to_le_bytes());
        data_offset += bytes.len() as u64;
    }
    for bytes in stored {
        out.extend_from_slice(&bytes);
    }
    out
}

fn stored(entry: &WriterEntry) -> &[u8] {
    match &entry.payload {
        Payload::Memory(bytes) => bytes,
        Payload::File { .. } => panic!("a modpkg chunk is held in memory"),
    }
}

#[test]
fn only_the_base_layer_of_each_wad_is_mounted_with_its_stored_bytes() {
    let bytes = package(
        &["base", "chroma"],
        &["zed.wad.client", "Champions/Map11.wad.client"],
        &[
            chunk(1, b"zed skin", true, 0, 0),
            chunk(2, b"zed bank", false, 0, 0),
            chunk(3, b"shared shadow", true, 0, 1),
            chunk(4, b"chroma only", true, 1, 0),
            chunk(5, b"thumbnail", false, NONE, NONE),
        ],
    );
    let package = ModPkg::read_index(Cursor::new(&bytes)).expect("index");
    assert_eq!(
        package.wad_names(),
        BTreeSet::from(["zed".to_owned(), "Map11".to_owned()])
    );

    let wads = package.base_wads(Cursor::new(&bytes)).expect("wads");
    let zed = &wads["zed"];
    assert_eq!(zed.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(zed[&1].kind, CompressionType::Zstd as u8);
    assert_eq!(
        zstd::decode_all(stored(&zed[&1])).expect("frame"),
        b"zed skin"
    );
    assert_eq!(zed[&1].checksum, content_checksum(stored(&zed[&1])));
    assert_eq!(zed[&2].kind, CompressionType::Raw as u8);
    assert_eq!(stored(&zed[&2]), b"zed bank");
    assert_eq!(wads["Map11"].keys().copied().collect::<Vec<_>>(), vec![3]);
}

#[test]
fn a_damaged_or_hostile_package_is_refused() {
    let good = package(
        &["base"],
        &["zed.wad.client"],
        &[chunk(1, b"zed skin", true, 0, 0)],
    );

    let mut corrupt = good.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xFF;
    let index = ModPkg::read_index(Cursor::new(&corrupt)).expect("index");
    assert!(
        index.base_wads(Cursor::new(&corrupt)).is_err(),
        "checksum mismatch"
    );

    let truncated = &good[..good.len() - 2];
    let index = ModPkg::read_index(Cursor::new(truncated)).expect("index");
    assert!(
        index.base_wads(Cursor::new(truncated)).is_err(),
        "chunk past the end"
    );

    let no_base = package(&["chroma"], &["zed.wad.client"], &[]);
    assert!(ModPkg::read_index(Cursor::new(&no_base)).is_err());

    for name in [
        "zed.dll",
        ".wad.client",
        "evil:stream.wad.client",
        "zed.wad",
    ] {
        let odd = package(&["base"], &[name], &[]);
        assert!(ModPkg::read_index(Cursor::new(&odd)).is_err(), "{name}");
    }

    let out_of_range = package(
        &["base"],
        &["zed.wad.client"],
        &[chunk(1, b"x", false, 0, 7)],
    );
    assert!(ModPkg::read_index(Cursor::new(&out_of_range)).is_err());

    assert!(ModPkg::read_index(Cursor::new(b"PK\x03\x04 not a modpkg")).is_err());
    assert!(is_modpkg(&good) && !is_modpkg(b"PK\x03\x04"));
}
