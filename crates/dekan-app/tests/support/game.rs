use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use dekan_app::mods_store;
use dekan_classic::generator::StandardChampion;
use dekan_core::mods::{ModCategory, ModSelection};
use dekan_inject::overlay_builder;
use dekan_wad::hash::{prop_key_hash, wad_path_hash};
use dekan_wad::prop::parse_prop_file;
use dekan_wad::wad::WadFile;
use dekan_wad::writer::WadWriter;

use super::*;

pub struct Game {
    pub dir: PathBuf,
    pub shadow_skin0: Vec<u8>,
}

pub fn game_install(root: &Path) -> Game {
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

pub fn fantome(path: &Path, files: &[(&str, Vec<u8>)]) {
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

pub struct Mounted {
    pub wads: BTreeMap<String, WadFile>,
    pub served: BTreeSet<String>,
}

impl Mounted {
    pub fn new(game: &Path, overlay: &Path) -> Self {
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

    pub fn read(&self, wad: &str, path: &str) -> Option<Vec<u8>> {
        self.wads[wad].read(wad_path_hash(path)).expect("read")
    }

    pub fn find(&self, path: &str) -> Vec<Vec<u8>> {
        self.wads
            .iter()
            .filter(|(name, _)| name.as_str() != MAP22)
            .filter_map(|(_, wad)| wad.read(wad_path_hash(path)).expect("read"))
            .collect()
    }
}

pub fn skin_fields(bin: &[u8]) -> (u32, Option<u32>, u32) {
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

pub fn reachable_objects(mounted: &Mounted, start: &str) -> Result<BTreeSet<u32>, String> {
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

pub struct Build {
    pub mods: Vec<String>,
    pub mods_dir: PathBuf,
}

pub async fn prepare(root: &Path, game: &Game, custom: Option<&[(&str, Vec<u8>)]>) -> Build {
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

pub fn build(game: &Game, prepared: &Build, overlay: &Path) -> overlay_builder::NativeBuild {
    overlay_builder::build(
        &game.dir,
        &prepared.mods_dir,
        overlay,
        &prepared.mods,
        &AtomicBool::new(false),
    )
    .expect("overlay build")
}
