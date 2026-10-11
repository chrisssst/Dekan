use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use dekan_inject::mod_compat::{Repairer, check, game_hash_set, mod_hashes, mod_wads};
use dekan_inject::overlay_builder::{GameWad, get_or_index_game};
use dekan_platform::fs::{ExtractLimits, safe_extract_zip};
use dekan_wad::WadFile;
use dekan_wad::hash::{relative_path_hash, wad_path_hash};
use dekan_wad::prop::tree::{self, Field, Value};
use dekan_wad::prop::{PropFile, parse_prop_file};
use dekan_wad::writer::{WadWriter, optimal_raw};

use crate::game_dir;

const LIMITS: ExtractLimits = ExtractLimits {
    max_total_bytes: 16 * 1024 * 1024 * 1024,
    max_single_file_bytes: 8 * 1024 * 1024 * 1024,
    max_entries: 200_000,
    max_path_len: dekan_platform::fs::MAX_PATH_CHARS,
};

const ASSET_EXTENSIONS: [&str; 11] = [
    ".tex", ".dds", ".skn", ".skl", ".anm", ".scb", ".sco", ".bnk", ".wpk", ".bin", ".png",
];

pub(crate) fn run_mod_audit(args: &[String]) {
    let mut root = None;
    let mut inputs = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--root" => root = iter.next().map(PathBuf::from),
            other => inputs.push(PathBuf::from(other)),
        }
    }
    let Some(game_root) = root.or_else(game_dir) else {
        return;
    };
    let mut archives = Vec::new();
    for input in inputs {
        if input.is_dir() {
            for entry in std::fs::read_dir(&input).into_iter().flatten().flatten() {
                let path = entry.path();
                let lower = path.to_string_lossy().to_ascii_lowercase();
                if lower.ends_with(".fantome") || lower.ends_with(".zip") {
                    archives.push(path);
                }
            }
        } else {
            archives.push(input);
        }
    }
    if archives.is_empty() {
        eprintln!("uso: cargo xtask mod-audit [--root <game>] <arquivo.fantome|pasta>...");
        std::process::exit(2);
    }

    let index = match get_or_index_game(&game_root) {
        Ok(index) => index,
        Err(e) => {
            eprintln!("jogo nao indexado: {e}");
            std::process::exit(1);
        }
    };
    let mut game = Game {
        hashes: game_hash_set(&index),
        index,
        opened: HashMap::new(),
        archive_keys: HashMap::new(),
        global_keys: None,
    };

    let work = std::env::temp_dir().join(format!("dekan_mod_audit_{}", std::process::id()));
    let mods_root = work.join("custom_mods");
    let skins = mods_root.join("skins");
    if let Err(e) = std::fs::create_dir_all(&skins) {
        eprintln!("pasta de trabalho: {e}");
        std::process::exit(1);
    }
    let mut copies = Vec::new();
    for archive in &archives {
        let Some(name) = archive.file_name() else {
            continue;
        };
        let copy = skins.join(name);
        if let Err(e) = std::fs::copy(archive, &copy) {
            eprintln!("{}: copia falhou: {e}", archive.display());
            continue;
        }
        copies.push((archive.clone(), copy));
    }

    println!(
        "jogo: {} | wads: {} | mods: {}",
        game_root.display(),
        game.index.len(),
        copies.len()
    );
    let repair = dekan_app::mod_repair::repair_custom_mods(
        std::slice::from_ref(&mods_root),
        &game_root,
        &work.join("state"),
        &|| false,
    );
    println!(
        "reparo do startup: verificados={} reparados={} incompativeis={}",
        repair.checked,
        repair.repaired.len(),
        repair.incompatible.len()
    );

    let mut failed = false;
    let format_hashes = game.hashes.clone();
    let format_index = game.index.clone();
    let mut formats = Repairer::new(&format_index, &format_hashes);
    for (original, copy) in &copies {
        let before_dir = work.join("before");
        let after_dir = work.join("after");
        if let Err(e) = extract(original, &before_dir).and_then(|()| extract(copy, &after_dir)) {
            println!("\n== {}\n   ilegivel: {e}", original.display());
            failed = true;
            continue;
        }
        let verdict = if repair.repaired.contains(copy) {
            "REPARADO"
        } else if repair.incompatible.contains(copy) {
            "INCOMPATIVEL"
        } else {
            "COMPATIVEL SEM MUDANCA"
        };
        println!("\n== {} -> {verdict}", original.display());
        let (before_wads, after_wads) = match wads(&before_dir, &work.join("packed_before"))
            .and_then(|b| wads(&after_dir, &work.join("packed_after")).map(|a| (b, a)))
        {
            Ok(pair) => pair,
            Err(e) => {
                println!("   ilegivel: {e}");
                failed = true;
                continue;
            }
        };
        let own_before = mod_hashes(
            &before_wads
                .values()
                .map(|w| w.source.clone())
                .collect::<Vec<_>>(),
        );
        let own_after = mod_hashes(
            &after_wads
                .values()
                .map(|w| w.source.clone())
                .collect::<Vec<_>>(),
        );
        for (name, after) in &after_wads {
            let Some(before) = before_wads.get(name) else {
                println!("   {name}: nao existe no original");
                failed = true;
                continue;
            };
            let report = match audit_wad(before, after, &own_before, &own_after, &mut game) {
                Ok(report) => report,
                Err(e) => {
                    println!("   {name}: ilegivel: {e}");
                    failed = true;
                    continue;
                }
            };
            println!("   {name}");
            match formats.unknown_formats(&after.source) {
                Ok(found) if found.is_empty() => {}
                Ok(found) => println!(
                    "     formatos de asset que o jogo nao usa mais: {}",
                    found
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Err(e) => println!("     formatos nao verificados: {e}"),
            }
            println!(
                "     links pendurados: antes={} depois={}",
                report.dangling_before,
                report.dangling_after.len()
            );
            for link in &report.dangling_after {
                println!("       pendurado: {link}");
            }
            println!(
                "     original x reparado: {} entradas mudaram; fora a lista de links e os caminhos de asset, tudo bit a bit igual: {}",
                report.changed_entries.len(),
                report.other_entries_identical
            );
            let distinct: BTreeSet<&(String, String)> = report.relinked.iter().collect();
            println!(
                "     links trocados: {} em {} bins ({} pares distintos)",
                report.relinked.len(),
                report.changed_entries.len(),
                distinct.len()
            );
            for (old, new) in distinct.iter().take(4) {
                println!("       {old}\n         -> {new}");
            }
            let assets: BTreeSet<&(String, String)> = report.asset_relinks.iter().collect();
            println!(
                "     caminhos de asset trocados: {} ({} distintos)",
                report.asset_relinks.len(),
                assets.len()
            );
            for (old, new) in assets.iter().take(3) {
                println!("       {old}\n         -> {new}");
            }
            println!(
                "     reparado x jogo: identicas={} diferentes={} so_no_mod={} (PROPs={})",
                report.same_as_game, report.differs_from_game, report.not_in_game, report.props
            );
            println!(
                "     referencias de objeto sem destino (alem das do proprio jogo): {} | bins linkados ausentes: {}",
                report.refs_only_in_mod.len(),
                report.linked_bins_missing
            );
            for r in report.refs_only_in_mod.iter().take(15) {
                println!("       objeto {r:08x}");
            }
            println!(
                "     arquivos referenciados: {} | sem destino no jogo nem no mod: {}",
                report.files_checked.len(),
                report.files_missing.len()
            );
            for f in report.files_missing.values().take(15) {
                println!("       arquivo {f}");
            }
            failed |= !report.dangling_after.is_empty()
                || !report.other_entries_identical
                || report.linked_bins_missing > 0;
        }
    }
    println!("\ncopias de trabalho em {}", work.display());
    if failed {
        std::process::exit(1);
    }
}

mod game;
mod props;
mod wad;

use game::{Game, ModWad, extract, wads};
use props::{closure_keys, prop_files, same_but_asset_paths, unresolved_refs};
use wad::audit_wad;
