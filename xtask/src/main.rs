mod audit;
mod build;
mod lint;
mod probes;
mod testing;

use std::path::PathBuf;
use std::process::ExitStatus;

use audit::{client_audit, install_audit, skin_audit};
use build::{check, package, version};
use lint::{adr008, comment_lexers, comments};
use probes::{app, classic, wad};
use testing::{fuzz, harness, smoke};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("help");

    match command {
        "check" => check::run_check(),
        "adr008" => adr008::run_adr008_check(),
        "comments" => comments::run(&args[2..]),
        "package" => package::run_package(),
        "installer" => package::run_installer(&args[2..]),
        "set-version" => version::run_set_version(&args[2..]),
        "client-probe" => app::run_client_probe(),
        "overlay-demo" => app::run_overlay_demo(),
        "library-probe" => app::run_library_probe(args.get(2).and_then(|a| a.parse().ok())),
        "catalog-demo" => {
            app::run_catalog_demo(args.get(2).and_then(|a| a.parse().ok()).unwrap_or(238))
        }
        "ipc-probe" => app::run_ipc_probe(args.get(2).and_then(|a| a.parse().ok()).unwrap_or(238)),
        "classic-probe" => classic::run_classic_probe(&args[2..]),
        "wad-probe" => wad::run_wad_probe(&args[2..]),
        "wad-writer-probe" => wad::run_wad_writer_probe(&args[2..]),
        "wad-types" => wad::run_wad_types(&args[2..]),
        "install-audit" => install_audit::run_install_audit(&args[2..]),
        "skin-audit" => skin_audit::run_skin_audit(&args[2..]),
        "prop-roundtrip" => wad::run_prop_roundtrip(&args[2..]),
        "client-dump" => client_audit::run_client_dump(&args[2..]),
        "client-audit" => client_audit::run_client_audit(&args[2..]),
        "harness" => harness::run_harness(&args[2..]),
        "smoke" => smoke::run_smoke(&args[2..]),
        "fuzz" => fuzz::run_fuzz(&args[2..]),
        _ => print_help(),
    }
}

fn print_help() {
    eprintln!("Dekan Automation Tool (xtask)\n");
    eprintln!("Usage: cargo xtask <command>\n");
    eprintln!("Commands:");
    eprintln!(
        "  set-version      - Stamp the workspace version in Cargo.toml and Cargo.lock (<major.minor>)"
    );
    eprintln!(
        "  client-probe     - Report the League Client window state and where the overlay would sit"
    );
    eprintln!(
        "  overlay-demo     - Show the overlay attached to the client for 30s (visual check)"
    );
    eprintln!(
        "  ipc-probe        - Click the real overlay from script and prove the choice reaches Rust"
    );
    eprintln!("  classic-probe    - Build Rift Classic mods from the installed game (<Alias...>)");
    eprintln!(
        "  wad-probe        - Decode every entry of the game WADs whose path matches <filter...>"
    );
    eprintln!("  wad-writer-probe - Copy real game WADs through WadWriter and check every entry");
    eprintln!(
        "  wad-types        - Count entry types from the tables of contents ([--root <folder>])"
    );
    eprintln!(
        "  install-audit    - After (un)install: files, hashes and registry as dekan.iss promises (installed | uninstalled [--kept-user-content])"
    );
    eprintln!(
        "  skin-audit       - Generate every skin of every champion from the installed game and report defects ([--root <game>] [--out <file>] [Alias...])"
    );
    eprintln!(
        "  client-audit     - Compare the installed client's skin data with the game and with Dekan's catalog ([--client <dir>] [--root <game>] [--out <file>])"
    );
    eprintln!(
        "  client-dump      - Print files of the client's game data (<client dir> <relative path...>)"
    );
    eprintln!(
        "  harness          - Generate skins, build real overlays and check them against the installed game ([--root <game>] [--out <file>])"
    );
    eprintln!(
        "  smoke            - Run a release dekan.exe without tools, with wrong tools and with the audited injector (<dekan.exe> <tools dir>)"
    );
    eprintln!(
        "  fuzz             - Mutate real game WADs and bins into the parsers; checks no panic and PROP round trips ([iterations] [seed])"
    );
    eprintln!(
        "  prop-roundtrip   - Read every object of every champion and map bin into the PROP tree and write it back; fails on any byte that differs ([--root <game>])"
    );
    eprintln!("  adr008           - Fail if any discarded Result lacks a `// ignore-ok: <reason>`");
    eprintln!(
        "  comments         - Fail on comments in code files; [--strip] removes them, proves the code is unchanged and lists the removed text ([--report <file>])"
    );
    eprintln!(
        "  check            - Validate workspace: error-handling sweep, no comments, fmt, clippy (-D warnings), test"
    );
    eprintln!("  package          - Build release profile and package binary into dist/");
    eprintln!(
        "  installer        - Build the Windows installer ([--prebuilt] keeps the already built and possibly signed dist/dekan.exe)"
    );
    eprintln!("  help             - Show this help message");
}

fn check_status(name: &str, status: ExitStatus) {
    if !status.success() {
        eprintln!("\n[ERROR] '{name}' failed with status: {status}");
        std::process::exit(1);
    }
}

fn game_dir() -> Option<PathBuf> {
    let found = dekan_platform::paths::discover_game_dir();
    if found.is_none() {
        println!(
            "Jogo nao encontrado: abra o cliente do League ou instale-o pelo Riot Client (ou use --root)"
        );
    }
    found
}

fn dekan_data_dir() -> PathBuf {
    match dekan_platform::paths::data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("[ERRO] pasta de dados do Dekan indisponivel: {e}");
            std::process::exit(1);
        }
    }
}

fn dekan_tools_dir() -> PathBuf {
    let candidates = dekan_platform::paths::tools_dir_candidates(&dekan_data_dir());
    candidates
        .iter()
        .find(|dir| dir.join("ltk_patcher_host.exe").is_file())
        .or_else(|| candidates.first())
        .cloned()
        .unwrap_or_default()
}
