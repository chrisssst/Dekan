use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use dekan_app::mods_store;
use dekan_classic::generator::StandardChampion;
use dekan_core::mods::{ModCategory, ModSelection};
use dekan_inject::{mod_compat, overlay_builder};
use dekan_wad::hash::{content_checksum, prop_key_hash, wad_path_hash};
use dekan_wad::prop::{PropEntry, PropFile, parse_prop_file, serialize_prop_file};
use dekan_wad::wad::WadFile;
use dekan_wad::writer::{Payload, WadWriter, WriterEntry};

const ZED: u32 = 238;
const SKIN_DATA: u32 = 0x9b67_e9f6;
const RESOURCES: u32 = 0xef3a_0f33;
const ANIMATION_GRAPH: u32 = 0xf5fb_07c7;
const CHAMPION_DATA: u32 = 0x45cd_899f;
const EMBED_ANIMATION: u32 = 0x1234_5678;
const FIELD_U32: u8 = 7;
const FIELD_STRING: u8 = 16;
const FIELD_EMBED: u8 = 0x83;
const FIELD_LINK: u8 = 0x84;

const CHAMPIONS: &str = "DATA/FINAL/Champions/Zed.wad.client";
const MAP11: &str = "DATA/FINAL/Maps/Shipping/Map11.wad.client";
const MAP22: &str = "DATA/FINAL/Maps/Shipping/Map22.wad.client";
const TEXTURE: &str = "assets/characters/zed/skins/skin69/zed_skin69_tx_cm.dds";
const NEW_TEXTURE: &str = "assets/characters/zed/skins/skin69/custom_glow.dds";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
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

fn field(body: &mut Vec<u8>, name: &str, kind: u8, value: &[u8]) {
    body.extend_from_slice(&prop_key_hash(name).to_le_bytes());
    body.push(kind);
    body.extend_from_slice(value);
}

fn string(value: &str) -> Vec<u8> {
    let mut out = u16::try_from(value.len())
        .expect("short string")
        .to_le_bytes()
        .to_vec();
    out.extend_from_slice(value.as_bytes());
    out
}

fn skin_object(
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

fn prop(links: &[String], entries: Vec<PropEntry>) -> Vec<u8> {
    serialize_prop_file(&PropFile {
        version: 3,
        links: links.to_vec(),
        entries,
    })
    .expect("prop")
}

fn skin_bin(character: &str, skin: u32, classification: Option<u32>, graph_skin: u32) -> Vec<u8> {
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

fn graph_bin(character: &str, skin: u32) -> Vec<u8> {
    prop(
        &[],
        vec![PropEntry {
            class_hash: ANIMATION_GRAPH,
            key_hash: prop_key_hash(&format!("Characters/{character}/Animations/Skin{skin}")),
            body: 0u16.to_le_bytes().to_vec(),
        }],
    )
}

fn character_bin(character: &str, companion: Option<&str>) -> Vec<u8> {
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

fn raw(bytes: &[u8]) -> WriterEntry {
    WriterEntry {
        kind: 0,
        subchunk_count: 0,
        first_subchunk: 0,
        uncompressed_size: bytes.len() as u64,
        checksum: content_checksum(bytes),
        payload: Payload::Memory(Arc::from(bytes.to_vec())),
    }
}

fn write_wad(path: &Path, signature: u8, files: &[(String, Vec<u8>)]) -> Vec<u8> {
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

struct Game {
    dir: PathBuf,
    shadow_skin0: Vec<u8>,
}

fn game_install(root: &Path) -> Game {
    let dir = root.join("Game");
    let shadow_skin0 = skin_bin("ZedShadow", 0, None, 0);
    let shadow_graph0 = graph_bin("ZedShadow", 0);
    let zed = [
        (
            "data/characters/zed/zed.bin".to_owned(),
            character_bin("Zed", Some("ZedShadow")),
        ),
        (
            "data/characters/zed/skins/skin0.bin".to_owned(),
            skin_bin("Zed", 0, Some(1), 0),
        ),
        (
            "data/characters/zed/skins/skin69.bin".to_owned(),
            skin_bin("Zed", 69, Some(1), 69),
        ),
        (
            "data/characters/zed/skins/skin70.bin".to_owned(),
            skin_bin("Zed", 70, Some(2), 69),
        ),
        (
            "data/characters/zed/animations/skin0.bin".to_owned(),
            graph_bin("Zed", 0),
        ),
        (
            "data/characters/zed/animations/skin69.bin".to_owned(),
            graph_bin("Zed", 69),
        ),
        (
            "data/characters/zedshadow/zedshadow.bin".to_owned(),
            character_bin("ZedShadow", None),
        ),
        (
            "data/characters/zedshadow/skins/skin0.bin".to_owned(),
            shadow_skin0.clone(),
        ),
        (
            "data/characters/zedshadow/skins/skin69.bin".to_owned(),
            skin_bin("ZedShadow", 69, None, 69),
        ),
        (
            "data/characters/zedshadow/skins/skin70.bin".to_owned(),
            skin_bin("ZedShadow", 70, None, 69),
        ),
        (
            "data/characters/zedshadow/animations/skin0.bin".to_owned(),
            shadow_graph0.clone(),
        ),
        (
            "data/characters/zedshadow/animations/skin69.bin".to_owned(),
            graph_bin("ZedShadow", 69),
        ),
        (TEXTURE.to_owned(), b"original skin 69 texture".to_vec()),
    ];
    write_wad(&dir.join(CHAMPIONS), 0x11, &zed);
    let map = [
        (
            "data/characters/zedshadow/skins/skin0.bin".to_owned(),
            shadow_skin0.clone(),
        ),
        (
            "data/characters/zedshadow/animations/skin0.bin".to_owned(),
            shadow_graph0.clone(),
        ),
        (
            "data/maps/shipping/map11/terrain.bin".to_owned(),
            b"summoner's rift terrain".to_vec(),
        ),
    ];
    write_wad(&dir.join(MAP11), 0x22, &map);
    write_wad(
        &dir.join(MAP22),
        0x33,
        &[(
            "data/characters/zedshadow/skins/skin0.bin".to_owned(),
            shadow_skin0.clone(),
        )],
    );
    std::fs::write(dir.join("League of Legends.exe"), b"exe").expect("exe");
    Game { dir, shadow_skin0 }
}

fn fantome(path: &Path, files: &[(&str, Vec<u8>)]) {
    let mut wad = WadWriter::default();
    for (name, bytes) in files {
        wad.insert(wad_path_hash(name), raw(bytes));
    }
    let wad_bytes = wad.to_bytes().expect("mod wad");
    let file = std::fs::File::create(path).expect("fantome");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("META/info.json", options).expect("info");
    zip.write_all(br#"{"Name":"Glow","Author":"test","Version":"1.0","Description":""}"#)
        .expect("info bytes");
    zip.start_file("WAD/Zed.wad.client", options)
        .expect("wad entry");
    zip.write_all(&wad_bytes).expect("wad bytes");
    zip.finish().expect("zip");
}

struct Mounted {
    wads: BTreeMap<String, WadFile>,
    served: BTreeSet<String>,
}

impl Mounted {
    fn new(game: &Path, overlay: &Path) -> Self {
        let mut wads = BTreeMap::new();
        let mut served = BTreeSet::new();
        for relative in [CHAMPIONS, MAP11, MAP22] {
            let from_overlay = overlay.join(relative);
            let path = if from_overlay.is_file() {
                served.insert(relative.to_owned());
                from_overlay
            } else {
                game.join(relative)
            };
            wads.insert(
                relative.to_owned(),
                WadFile::open(&path).expect("mounted wad"),
            );
        }
        Self { wads, served }
    }

    fn read(&self, wad: &str, path: &str) -> Option<Vec<u8>> {
        self.wads[wad].read(wad_path_hash(path)).expect("read")
    }

    fn find(&self, path: &str) -> Vec<Vec<u8>> {
        self.wads
            .iter()
            .filter(|(name, _)| name.as_str() != MAP22)
            .filter_map(|(_, wad)| wad.read(wad_path_hash(path)).expect("read"))
            .collect()
    }
}

fn skin_fields(bin: &[u8]) -> (u32, Option<u32>, u32) {
    let parsed = parse_prop_file(bin).expect("skin bin");
    let skin = parsed
        .entries
        .iter()
        .find(|e| e.class_hash == SKIN_DATA)
        .expect("skin object");
    let body = &skin.body;
    let classification_hash = prop_key_hash("skinClassification").to_le_bytes();
    let classification = (body[2..6] == classification_hash)
        .then(|| u32::from_le_bytes(body[7..11].try_into().expect("u32")));
    let graph_hash = prop_key_hash("animationGraphData").to_le_bytes();
    let at = body
        .windows(4)
        .position(|w| w == graph_hash)
        .expect("animation graph field");
    let graph = u32::from_le_bytes(body[at + 5..at + 9].try_into().expect("link"));
    (skin.key_hash, classification, graph)
}

fn reachable_objects(mounted: &Mounted, start: &str) -> Result<BTreeSet<u32>, String> {
    let mut objects = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([start.to_ascii_lowercase()]);
    while let Some(path) = queue.pop_front() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(bytes) = mounted.find(&path).into_iter().next() else {
            return Err(format!("link '{path}' is not in any mounted WAD"));
        };
        let parsed = parse_prop_file(&bytes).map_err(|e| format!("{path}: {e}"))?;
        objects.extend(parsed.entries.iter().map(|e| e.key_hash));
        queue.extend(parsed.links.iter().map(|l| l.to_ascii_lowercase()));
    }
    Ok(objects)
}

struct Build {
    mods: Vec<String>,
    mods_dir: PathBuf,
}

async fn prepare(root: &Path, game: &Game, custom: Option<&[(&str, Vec<u8>)]>) -> Build {
    let app_data = root.join("Dekan");
    let mods_dir = app_data.join("mods");
    std::fs::create_dir_all(&mods_dir).expect("mods dir");

    let generated = StandardChampion::open(&game.dir, "Zed")
        .expect("champion")
        .with_cache_dir(&app_data.join("state"))
        .build_mod(70, Some(69), &mods_dir)
        .expect("generated skin");
    let mut mods = vec![generated];

    if let Some(files) = custom {
        let source = root.join("Glow.fantome");
        fantome(&source, files);
        let roots = mods_store::mod_roots(&app_data);
        mods_store::import_archive(&roots[0].path, ModCategory::Skin, Some(ZED), &source)
            .expect("import");
        let catalog = mods_store::load_mod_catalog(roots, Some(ZED), Some("Zed".into())).await;
        let entry = catalog.skin.first().expect("imported mod listed for Zed");
        let selection = ModSelection {
            skin: BTreeMap::from([(ZED, entry.id.clone())]),
            ..ModSelection::default()
        };
        mods.extend(mods_store::stage_selected(
            &catalog,
            &selection,
            Some(ZED),
            &mods_dir,
        ));
    }
    Build { mods, mods_dir }
}

fn build(game: &Game, prepared: &Build, overlay: &Path) -> overlay_builder::NativeBuild {
    overlay_builder::build(
        &game.dir,
        &prepared.mods_dir,
        overlay,
        &prepared.mods,
        &AtomicBool::new(false),
    )
    .expect("overlay build")
}

#[tokio::test]
async fn a_generated_chroma_loads_completely_in_what_the_game_would_mount() {
    let root = Scratch::new("chroma");
    let game = game_install(&root.0);
    let prepared = prepare(&root.0, &game, None).await;
    let overlay = root.0.join("Dekan").join("overlay");
    build(&game, &prepared, &overlay);
    let mounted = Mounted::new(&game.dir, &overlay);

    assert_eq!(
        mounted.served,
        BTreeSet::from([CHAMPIONS.to_owned(), MAP11.to_owned()]),
        "the champion WAD and the map that shares the shadow are served; the TFT map never is"
    );

    let skin0 = mounted
        .read(CHAMPIONS, "data/characters/zed/skins/skin0.bin")
        .expect("zed skin0.bin");
    let (key, classification, graph) = skin_fields(&skin0);
    assert_eq!(
        key,
        prop_key_hash("Characters/Zed/Skins/Skin0"),
        "the chroma sits in slot 0"
    );
    assert_eq!(
        classification,
        Some(1),
        "slot 0 is classified as a base skin"
    );
    assert_eq!(
        graph,
        prop_key_hash("Characters/Zed/Animations/Skin69"),
        "the chroma keeps its own animation graph"
    );

    let game_skin70 = WadFile::open(&game.dir.join(CHAMPIONS))
        .expect("game")
        .read(wad_path_hash("data/characters/zed/skins/skin70.bin"))
        .expect("read")
        .expect("skin70");
    let source = parse_prop_file(&game_skin70).expect("skin70");
    let generated = parse_prop_file(&skin0).expect("skin0");
    assert_eq!(generated.links[0], "DATA/Characters/Zed/Skins/Skin70.bin");
    assert_eq!(
        &generated.links[1..],
        &source.links[..],
        "every dependency of the source is kept"
    );
    for (made, original) in generated.entries.iter().zip(&source.entries) {
        let mut expected = original.body.clone();
        if made.class_hash == SKIN_DATA {
            expected[7..11].copy_from_slice(&1u32.to_le_bytes());
        }
        assert_eq!(
            made.body, expected,
            "object {:#x} is the game's bytes",
            made.class_hash
        );
    }

    let reachable = reachable_objects(&mounted, "data/characters/zed/skins/skin0.bin")
        .unwrap_or_else(|e| panic!("asset resolution failed: {e}"));
    assert!(
        reachable.contains(&graph),
        "the animation graph the skin names is defined in a mounted, linked bin"
    );

    let shadow = mounted.find("data/characters/zedshadow/skins/skin0.bin");
    assert_eq!(
        shadow.len(),
        2,
        "the shadow is in the champion WAD and in Map11"
    );
    assert_eq!(
        shadow[0], shadow[1],
        "both mounted WADs agree on the shadow"
    );
    assert_ne!(
        shadow[0], game.shadow_skin0,
        "the shadow takes the skin's look"
    );
    let (shadow_key, _, shadow_graph) = skin_fields(&shadow[0]);
    assert_eq!(
        shadow_key,
        prop_key_hash("Characters/ZedShadow/Skins/Skin0")
    );
    assert_eq!(
        shadow_graph,
        prop_key_hash("Characters/ZedShadow/Animations/Skin69")
    );
    reachable_objects(&mounted, "data/characters/zedshadow/skins/skin0.bin")
        .unwrap_or_else(|e| panic!("shadow asset resolution failed: {e}"));

    for (wad, path) in [
        (CHAMPIONS, "data/characters/zed/animations/skin0.bin"),
        (CHAMPIONS, "data/characters/zedshadow/animations/skin0.bin"),
        (MAP11, "data/characters/zedshadow/animations/skin0.bin"),
    ] {
        let original = WadFile::open(&game.dir.join(wad))
            .expect("game")
            .read(wad_path_hash(path))
            .expect("read");
        assert_eq!(
            mounted.read(wad, path),
            original,
            "{path} in {wad}: the base graph is untouched"
        );
    }
    assert_eq!(
        mounted.read(MAP22, "data/characters/zedshadow/skins/skin0.bin"),
        Some(game.shadow_skin0.clone())
    );
}

#[tokio::test]
async fn rebuilt_wads_keep_the_games_header_and_every_untouched_byte() {
    let root = Scratch::new("faithful");
    let game = game_install(&root.0);
    let prepared = prepare(&root.0, &game, None).await;
    let overlay = root.0.join("Dekan").join("overlay");
    build(&game, &prepared, &overlay);

    for relative in [CHAMPIONS, MAP11] {
        let original = std::fs::read(game.dir.join(relative)).expect("game wad");
        let rebuilt = std::fs::read(overlay.join(relative)).expect("overlay wad");
        assert_eq!(
            &rebuilt[..268],
            &original[..268],
            "{relative}: signature and checksum are the game's"
        );
        let game_wad = WadFile::open(&game.dir.join(relative)).expect("game");
        let built = WadFile::open(&overlay.join(relative)).expect("built");
        let changed = [
            wad_path_hash("data/characters/zed/skins/skin0.bin"),
            wad_path_hash("data/characters/zedshadow/skins/skin0.bin"),
        ];
        for entry in game_wad.toc() {
            let rebuilt_entry = built.entry(entry.path_hash).expect("entry kept");
            if changed.contains(&entry.path_hash) {
                continue;
            }
            assert_eq!(
                built.read_raw(rebuilt_entry).expect("raw"),
                game_wad.read_raw(entry).expect("raw"),
                "{relative}: {:#x} is copied byte for byte",
                entry.path_hash
            );
        }
    }
}

#[tokio::test]
async fn an_imported_custom_skin_mod_is_applied_after_the_generated_skin() {
    let root = Scratch::new("custom");
    let game = game_install(&root.0);
    let custom = [
        (TEXTURE, b"custom glowing texture".to_vec()),
        (NEW_TEXTURE, b"brand new glow layer".to_vec()),
    ];
    let prepared = prepare(&root.0, &game, Some(&custom)).await;
    assert_eq!(
        prepared.mods.len(),
        2,
        "the generated skin and the staged custom mod"
    );

    let game_hashes =
        mod_compat::game_hash_set(&overlay_builder::get_or_index_game(&game.dir).expect("index"));
    for name in &prepared.mods[1..] {
        let wad = prepared
            .mods_dir
            .join(name)
            .join("WAD")
            .join("Zed.wad.client");
        let compat = mod_compat::check_wad(&wad, &game_hashes).expect("compat");
        assert!(compat.is_compatible(), "{name}: {:?}", compat.dangling);
    }

    let overlay = root.0.join("Dekan").join("overlay");
    build(&game, &prepared, &overlay);
    let mounted = Mounted::new(&game.dir, &overlay);
    assert_eq!(
        mounted.read(CHAMPIONS, TEXTURE).as_deref(),
        Some(&b"custom glowing texture"[..])
    );
    assert_eq!(
        mounted.read(CHAMPIONS, NEW_TEXTURE).as_deref(),
        Some(&b"brand new glow layer"[..])
    );
    let (key, classification, _) = skin_fields(
        &mounted
            .read(CHAMPIONS, "data/characters/zed/skins/skin0.bin")
            .expect("skin0"),
    );
    assert_eq!(
        (key, classification),
        (prop_key_hash("Characters/Zed/Skins/Skin0"), Some(1))
    );
    let shadow = mounted.find("data/characters/zedshadow/skins/skin0.bin");
    assert_eq!(
        shadow[0], shadow[1],
        "adding a custom mod keeps the map consistent"
    );
}

#[tokio::test]
async fn a_custom_mod_linking_a_missing_bin_is_reported_as_dangling() {
    let root = Scratch::new("dangling");
    let game = game_install(&root.0);
    let broken = prop(
        &["DATA/Characters/Zed/Skins/Skin999.bin".to_owned()],
        vec![PropEntry {
            class_hash: SKIN_DATA,
            key_hash: prop_key_hash("Characters/Zed/Skins/Skin0"),
            body: 0u16.to_le_bytes().to_vec(),
        }],
    );
    let prepared = prepare(
        &root.0,
        &game,
        Some(&[("data/characters/zed/skins/skin0.bin", broken)]),
    )
    .await;
    let game_hashes =
        mod_compat::game_hash_set(&overlay_builder::get_or_index_game(&game.dir).expect("index"));
    let wad = prepared
        .mods_dir
        .join(&prepared.mods[1])
        .join("WAD")
        .join("Zed.wad.client");
    let compat = mod_compat::check_wad(&wad, &game_hashes).expect("compat");
    assert!(
        !compat.is_compatible(),
        "a link to a bin the patch lacks must be caught before the game"
    );
}

#[tokio::test]
async fn the_overlay_is_deterministic_and_an_identical_rebuild_writes_nothing() {
    let first_root = Scratch::new("determinism_a");
    let second_root = Scratch::new("determinism_b");
    let mut outputs: Vec<HashMap<&str, Vec<u8>>> = Vec::new();
    for root in [&first_root, &second_root] {
        let game = game_install(&root.0);
        let prepared = prepare(&root.0, &game, None).await;
        let overlay = root.0.join("Dekan").join("overlay");
        build(&game, &prepared, &overlay);
        let again = build(&game, &prepared, &overlay);
        assert_eq!(again.written, 0, "an identical rebuild rewrites nothing");
        outputs.push(
            [CHAMPIONS, MAP11]
                .into_iter()
                .map(|relative| {
                    (
                        relative,
                        std::fs::read(overlay.join(relative)).expect("overlay"),
                    )
                })
                .collect(),
        );
    }
    assert_eq!(
        outputs[0], outputs[1],
        "the same fixture always gives the same bytes"
    );
}
