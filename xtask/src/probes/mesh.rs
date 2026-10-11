use crate::game_dir;
use std::path::PathBuf;

const SKN_MAGIC: &[u8] = &[0x33, 0x22, 0x11, 0x00];
const SKL_TOKEN: &[u8] = &[0xC3, 0x4F, 0xFD, 0x22];

#[derive(Default)]
struct Tally {
    checked: usize,
    other_version: usize,
    differs: usize,
    most_joints: usize,
    most_influences: usize,
    examples: Vec<String>,
}

impl Tally {
    fn record(&mut self, same: bool, what: impl FnOnce() -> String) {
        self.checked += 1;
        if !same {
            self.differs += 1;
            if self.examples.len() < 10 {
                self.examples.push(what());
            }
        }
    }
}

pub(crate) fn run_mesh_roundtrip(args: &[String]) {
    let root = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|p| args.get(p + 1))
        .map(PathBuf::from);
    let Some(game) = root.or_else(game_dir) else {
        println!("mesh-roundtrip: no game folder found; pass --root <game>");
        std::process::exit(1);
    };
    let final_dir = game.join("DATA").join("FINAL");
    let mut wads: Vec<PathBuf> = ["Champions", "Maps/Shipping"]
        .iter()
        .filter_map(|dir| std::fs::read_dir(final_dir.join(dir)).ok())
        .flat_map(|entries| entries.flatten().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".wad.client"))
        .collect();
    wads.sort();
    let (mut meshes, mut skeletons) = (Tally::default(), Tally::default());
    let (mut unreadable, mut unreadable_entries) = (0usize, 0usize);
    for path in &wads {
        let Ok(wad) = dekan_wad::wad::WadFile::open(path) else {
            unreadable += 1;
            continue;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let hashes: Vec<u64> = wad.entries().map(|(hash, _)| hash).collect();
        for hash in hashes {
            let bytes = match wad.read(hash) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => continue,
                Err(_) => {
                    unreadable_entries += 1;
                    continue;
                }
            };
            let what = || format!("{name} entry {hash:016x}");
            if bytes.starts_with(SKN_MAGIC) {
                match dekan_classic::skinned_mesh::parse(&bytes) {
                    Ok(mesh) => {
                        let same = dekan_classic::skinned_mesh::write(&mesh)
                            .is_ok_and(|written| written == bytes);
                        meshes.record(same, what);
                    }
                    Err(_) if bytes.get(4..6) != Some(&[4, 0]) => meshes.other_version += 1,
                    Err(e) => meshes.record(false, || format!("{}: {e}", what())),
                }
            } else if bytes.get(4..8) == Some(SKL_TOKEN) {
                match dekan_classic::skeleton::parse(&bytes) {
                    Ok(skeleton) => {
                        skeletons.most_joints = skeletons.most_joints.max(skeleton.joints.len());
                        skeletons.most_influences =
                            skeletons.most_influences.max(skeleton.influences.len());
                        let same = dekan_classic::skeleton::write(&skeleton).is_ok_and(|written| {
                            written.len() == bytes.len()
                                && dekan_classic::skeleton::parse(&written)
                                    .is_ok_and(|parsed| parsed == skeleton)
                        });
                        skeletons.record(same, what);
                    }
                    Err(e) => skeletons.record(false, || format!("{}: {e}", what())),
                }
            }
        }
    }
    println!(
        "wads: {} ({unreadable} unreadable, {unreadable_entries} entries unreadable) | skn v4: {} (other versions skipped: {}) not identical: {} | skl: {} not identical: {} | most joints: {} | most influences: {}",
        wads.len(),
        meshes.checked,
        meshes.other_version,
        meshes.differs,
        skeletons.checked,
        skeletons.differs,
        skeletons.most_joints,
        skeletons.most_influences
    );
    for example in meshes.examples.iter().chain(&skeletons.examples) {
        println!("  {example}");
    }
    if meshes.differs + skeletons.differs + unreadable_entries > 0 {
        std::process::exit(1);
    }
}
