mod client_audit;
mod comment_lexers;
mod comments;
mod fuzz;
mod harness;
mod install_audit;
mod skin_audit;
mod smoke;

use std::path::PathBuf;
use std::process::{Command, ExitStatus};

use sha2::{Digest, Sha256};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("help");

    match command {
        "check" => run_check(),
        "adr008" => run_adr008_check(),
        "comments" => comments::run(&args[2..]),
        "package" => run_package(),
        "installer" => run_installer(&args[2..]),
        "client-probe" => run_client_probe(),
        "overlay-demo" => run_overlay_demo(),
        "library-probe" => run_library_probe(args.get(2).and_then(|a| a.parse().ok())),
        "catalog-demo" => run_catalog_demo(args.get(2).and_then(|a| a.parse().ok()).unwrap_or(238)),
        "ipc-probe" => run_ipc_probe(args.get(2).and_then(|a| a.parse().ok()).unwrap_or(238)),
        "classic-probe" => run_classic_probe(&args[2..]),
        "wad-probe" => run_wad_probe(&args[2..]),
        "wad-writer-probe" => run_wad_writer_probe(&args[2..]),
        "wad-types" => run_wad_types(&args[2..]),
        "install-audit" => run_install_audit(&args[2..]),
        "skin-audit" => run_skin_audit(&args[2..]),
        "prop-roundtrip" => run_prop_roundtrip(&args[2..]),
        "client-dump" => run_client_dump(&args[2..]),
        "client-audit" => run_client_audit(&args[2..]),
        "harness" => run_harness(&args[2..]),
        "smoke" => run_smoke(&args[2..]),
        "fuzz" => run_fuzz(&args[2..]),
        _ => print_help(),
    }
}

fn print_help() {
    eprintln!("Dekan Automation Tool (xtask)\n");
    eprintln!("Usage: cargo xtask <command>\n");
    eprintln!("Commands:");
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

fn run_check() {
    println!(
        "==> Step 0/4: Error-handling sweep (every discarded Result justified) and no comments in code..."
    );
    run_adr008_check();
    comments::run(&[]);

    println!("==> Step 1/4: Checking formatting (cargo fmt)...");
    let status = Command::new("cargo")
        .args(["fmt", "--all", "--", "--check"])
        .status()
        .expect("failed to execute cargo fmt");
    check_status("cargo fmt", status);

    println!("==> Step 2/4: Running clippy (cargo clippy -D warnings)...");
    let status = Command::new("cargo")
        .args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .status()
        .expect("failed to execute cargo clippy");
    check_status("cargo clippy", status);

    println!("==> Step 3/4: Running workspace test suite (cargo test)...");
    let status = Command::new("cargo")
        .args(["test", "--workspace"])
        .status()
        .expect("failed to execute cargo test");
    check_status("cargo test", status);

    println!("\n[OK] All workspace checks passed with 100% success!");
}

fn release_rustflags(workspace_root: &std::path::Path) -> String {
    let mut flags: Vec<String> = vec!["-C".into(), "target-feature=+crt-static".into()];
    if let Ok(extra) = std::env::var("RUSTFLAGS") {
        flags.extend(extra.split_whitespace().map(str::to_owned));
    }
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".cargo")));
    if let Some(home) = cargo_home {
        flags.push(format!("--remap-path-prefix={}=cargo", home.display()));
    }
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .current_dir(workspace_root)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    if let Some(sysroot) = sysroot {
        flags.push(format!("--remap-path-prefix={sysroot}=rust"));
    }
    flags.push(format!(
        "--remap-path-prefix={}=dekan",
        workspace_root.display()
    ));
    flags.join("\u{1f}")
}

fn run_package() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    println!("==> Compiling optimized release binary...");
    let status = Command::new("cargo")
        .args(["build", "--release", "--locked"])
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            release_rustflags(&workspace_root),
        )
        .env_remove("RUSTFLAGS")
        .status()
        .expect("failed to compile release");
    check_status("cargo build --release", status);

    let candidate_paths = [
        workspace_root.join("target/x86_64-pc-windows-msvc/release/dekan.exe"),
        workspace_root.join("target/release/dekan.exe"),
    ];

    let binary_path = candidate_paths
        .iter()
        .find(|p| p.exists())
        .expect("release binary not found");

    let dist_dir = workspace_root.join("dist");
    clean_dist(&dist_dir);

    let target_dest = dist_dir.join("dekan.exe");
    std::fs::copy(binary_path, &target_dest).expect("failed to copy binary to dist/");

    let bytes = std::fs::read(&target_dest).expect("read binary");
    let hash_hex = dekan_inject::dll_validator::to_hex(&Sha256::digest(&bytes));

    let mut checksum_content = format!("{hash_hex} *dekan.exe\n");

    let relay_paths = [
        workspace_root.join("target/x86_64-pc-windows-msvc/release/dekan-relay.exe"),
        workspace_root.join("target/release/dekan-relay.exe"),
    ];
    if let Some(relay_bin) = relay_paths.iter().find(|p| p.exists()) {
        let relay_dest = dist_dir.join("dekan-relay.exe");
        if std::fs::copy(relay_bin, &relay_dest).is_ok() {
            if let Ok(bytes_relay) = std::fs::read(&relay_dest) {
                let hash_relay =
                    dekan_inject::dll_validator::to_hex(&Sha256::digest(&bytes_relay));
                checksum_content.push_str(&format!("{hash_relay} *dekan-relay.exe\n"));
                println!(
                    "  Relay:    {} ({} bytes, SHA-256: {})",
                    relay_dest.display(),
                    bytes_relay.len(),
                    hash_relay
                );
            }
        }
    }

    let default_party_json = format!(
        "{{\n  \"relay_url\": \"{}\"\n}}\n",
        dekan_party::config::DEFAULT_RELAY_URL
    );
    std::fs::write(dist_dir.join("party.json"), default_party_json)
        .expect("write default party.json");
    println!(
        "  Party:    dist/party.json (default: {})",
        dekan_party::config::DEFAULT_RELAY_URL
    );

    std::fs::write(dist_dir.join("SHA256SUMS"), &checksum_content).expect("write checksum file");

    println!("\n[OK] Package created successfully in dist/!");
    println!("  Binary:   {}", target_dest.display());
    println!("  Size:     {} bytes", bytes.len());
    println!("  SHA-256:  {hash_hex}");
}

const USER_SUPPLIED_TOOLS: [(&str, &[&str]); 2] = [
    ("ltk_patcher_host.exe", &["AUDITED_LTK_HOST_HASH"]),
    ("ltk_patcher_dll.dll", &["AUDITED_LTK_DLL_HASH"]),
];

const TRIGGER_SOURCE: &str = include_str!("../../crates/dekan-app/src/trigger.rs");

fn audited_hash(name: &str) -> Result<&'static str, String> {
    let declaration = format!("pub const {name}: &str =");
    let after = TRIGGER_SOURCE
        .split_once(declaration.as_str())
        .map(|(_, rest)| rest)
        .ok_or_else(|| format!("{name} is not declared in crates/dekan-app/src/trigger.rs"))?;
    let hash = after
        .trim_start()
        .strip_prefix('"')
        .and_then(|rest| rest.split_once('"'))
        .map(|(hash, _)| hash)
        .ok_or_else(|| {
            format!("{name} in crates/dekan-app/src/trigger.rs is not a string literal")
        })?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{name} is not a SHA-256 hex digest: {hash:?}"));
    }
    Ok(hash)
}

fn clean_dist(dist_dir: &std::path::Path) {
    if !dist_dir.exists() {
        std::fs::create_dir_all(dist_dir).expect("failed to create dist directory");
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dist_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "tools" || name == "library" {
                continue;
            }
            if path.is_file() {
                // ignore-ok: clean old artifact
                let _ = std::fs::remove_file(&path);
            } else if path.is_dir() && name == "installer" {
                // ignore-ok: clean old installer folder
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }
}

fn run_installer(args: &[String]) {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    if args.iter().any(|a| a == "--prebuilt") {
        for file in ["dekan.exe", "party.json"] {
            if !workspace_root.join("dist").join(file).is_file() {
                eprintln!(
                    "[ERROR] --prebuilt: dist/{file} ausente; rode `cargo xtask package` antes."
                );
                std::process::exit(1);
            }
        }
        println!("==> Using the prebuilt dist/dekan.exe");
    } else {
        run_package();
    }

    let iscc_candidates: Vec<PathBuf> = std::env::var_os("ISCC")
        .map(PathBuf::from)
        .into_iter()
        .chain(
            ["ProgramFiles(x86)", "ProgramFiles"]
                .iter()
                .filter_map(std::env::var_os)
                .map(|dir| PathBuf::from(dir).join("Inno Setup 6").join("ISCC.exe")),
        )
        .collect();

    let Some(iscc) = iscc_candidates.iter().find(|p| p.exists()) else {
        eprintln!("[ERROR] ISCC.exe não encontrado. Instale o Inno Setup 6.");
        std::process::exit(1);
    };

    let script = workspace_root.join("installer").join("dekan.iss");
    println!("==> Building installer with {}...", iscc.display());

    let raw_version = env!("CARGO_PKG_VERSION");
    let version = raw_version.strip_suffix(".0").unwrap_or(raw_version);
    let status = Command::new(iscc)
        .arg(format!("/DMyAppVersion={version}"))
        .arg(&script)
        .status()
        .expect("failed to run ISCC");
    check_status("ISCC", status);
    println!("  Version:  {version} (display format from Cargo.toml)");

    let output = workspace_root.join(format!("dist/installer/Dekan-Setup-{version}-x64.exe"));
    match std::fs::metadata(&output) {
        Ok(meta) => println!(
            "
[OK] Installer: {} ({} bytes)",
            output.display(),
            meta.len()
        ),
        Err(e) => {
            eprintln!("[ERROR] installer not produced: {e}");
            std::process::exit(1);
        }
    }
}

fn run_adr008_check() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    let mut offenders = Vec::new();
    let mut checked = 0usize;
    let mut justified = 0usize;

    for dir in ["crates", "xtask"] {
        collect_rust_files(&workspace_root.join(dir), &mut |path| {
            let Ok(content) = std::fs::read_to_string(path) else {
                return;
            };
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !line.trim_start().starts_with("let _ = ") {
                    continue;
                }
                checked += 1;
                let previous = i.checked_sub(1).map(|p| lines[p]).unwrap_or("");
                if line.contains("ignore-ok") || previous.trim_start().starts_with("// ignore-ok") {
                    justified += 1;
                    continue;
                }
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        });
    }

    if offenders.is_empty() {
        println!("  {checked} discarded Results, {justified} justified, 0 unexplained");
        return;
    }

    eprintln!(
        "
[ERROR] Error-handling sweep: {} discarded Result(s) without a `// ignore-ok: <reason>`:",
        offenders.len()
    );
    for offender in &offenders {
        eprintln!("  {offender}");
    }
    std::process::exit(1);
}

fn collect_rust_files(dir: &std::path::Path, visit: &mut impl FnMut(&std::path::Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rust_files(&path, visit);
        } else if path.extension().is_some_and(|e| e == "rs") {
            visit(&path);
        }
    }
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

fn run_fuzz(args: &[String]) {
    let iterations = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(200_000usize);
    let seed = args
        .get(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(0x5eed_b011e7u64);
    let Some(game) = game_dir() else {
        return;
    };
    let corpus = fuzz::corpus(
        &game,
        &[
            ("Zed", 10),
            ("Orianna", 1),
            ("Annie", 5),
            ("Lux", 7),
            ("Garen", 44),
            ("Seraphine", 2),
            ("Kaisa", 71),
            ("MonkeyKing", 3),
            ("Yasuo", 87),
            ("Sona", 6),
        ],
    );
    println!(
        "corpus: {} WADs reais reduzidos, {} bins reais | iteracoes: {iterations} | semente: {seed}",
        corpus.wads.len(),
        corpus.bins.len()
    );
    let started = std::time::Instant::now();
    let report = fuzz::run(&corpus, iterations, seed);
    println!(
        "{} iteracoes em {} s: {} aceitas, {} recusadas com erro tipado, {} panicos",
        report.iterations,
        started.elapsed().as_secs(),
        report.accepted,
        report.rejected,
        report.panics.len()
    );
    println!(
        "propriedades em bins reais: {} verificacoes, {} falhas",
        report.property_checks,
        report.property_failures.len()
    );
    for failure in report
        .property_failures
        .iter()
        .chain(report.panics.iter())
        .take(20)
    {
        println!("  FALHA {failure}");
    }
    if !report.panics.is_empty() || !report.property_failures.is_empty() {
        std::process::exit(1);
    }
}

fn run_smoke(args: &[String]) {
    let exe = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dist").join("dekan.exe"));
    let Some(tools) = args.get(1).map(PathBuf::from) else {
        println!("uso: smoke <dekan.exe> <pasta com o injetor auditado>");
        return;
    };
    if !exe.is_file() {
        println!("binario nao encontrado: {}", exe.display());
        std::process::exit(1);
    }
    let checks = smoke::run(&exe, &tools);
    let failed = checks.iter().filter(|c| !c.ok).count();
    println!(
        "
smoke: {} verificacoes, {failed} falhas",
        checks.len()
    );
    for c in checks.iter().filter(|c| !c.ok) {
        println!("  FALHA {}: {}", c.scenario, c.expectation);
    }
    if failed > 0 {
        std::process::exit(1);
    }
}

fn run_harness(args: &[String]) {
    let mut root = None;
    let mut out = std::env::temp_dir().join("dekan_harness.md");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--root" => root = iter.next().map(PathBuf::from),
            "--out" => {
                if let Some(path) = iter.next() {
                    out = PathBuf::from(path);
                }
            }
            other => println!("argumento ignorado: {other}"),
        }
    }
    let Some(game) = root.or_else(game_dir) else {
        return;
    };
    let data = match dekan_classic::client_data::ClientGameData::for_game(&game) {
        Ok(data) => data,
        Err(e) => {
            println!("dados do cliente indisponiveis: {e}");
            return;
        }
    };
    let cases = match harness::sample(&data, &game) {
        Ok(cases) => cases,
        Err(e) => {
            println!("{e}");
            return;
        }
    };
    let scratch = std::env::temp_dir().join("dekan_harness_run");
    let cache = std::env::temp_dir().join("dekan_harness_cache");
    if let Err(e) = std::fs::create_dir_all(&cache) {
        println!("pasta temporaria indisponivel: {e}");
        return;
    }
    println!("jogo: {} | casos: {}", game.display(), cases.len());
    let mut outcomes = Vec::with_capacity(cases.len());
    for (n, case) in cases.iter().enumerate() {
        let outcome = harness::run_case(case, &game, &scratch, &cache);
        println!(
            "[{}/{}] {} {} ({} ms)",
            n + 1,
            cases.len(),
            if outcome.failures.is_empty() {
                "ok"
            } else {
                "FALHA"
            },
            outcome.label,
            outcome.build_ms
        );
        outcomes.push(outcome);
    }
    match std::fs::write(&out, harness::render(&outcomes)) {
        Ok(()) => println!("relatorio: {}", out.display()),
        Err(e) => println!("relatorio nao gravado em {}: {e}", out.display()),
    }
    if outcomes.iter().any(|o| !o.failures.is_empty()) {
        std::process::exit(1);
    }
}

fn run_client_audit(args: &[String]) {
    let mut client_dir = None;
    let mut root = None;
    let mut out = std::env::temp_dir().join("dekan_client_audit.md");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--client" => client_dir = iter.next().map(PathBuf::from),
            "--root" => root = iter.next().map(PathBuf::from),
            "--out" => {
                if let Some(path) = iter.next() {
                    out = PathBuf::from(path);
                }
            }
            other => println!("argumento ignorado: {other}"),
        }
    }
    let Some(game) = root.or_else(game_dir) else {
        return;
    };
    let client_dir = client_dir
        .or_else(|| game.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let data = match client_audit::ClientData::open(&client_dir) {
        Ok(data) => data,
        Err(e) => {
            println!("dados do cliente indisponiveis: {e}");
            return;
        }
    };
    let champions = match client_audit::champion_ids(&data) {
        Ok(champions) => champions,
        Err(e) => {
            println!("{e}");
            return;
        }
    };
    let regular: Vec<&(i64, String)> = champions
        .iter()
        .filter(|(id, _)| !client_audit::is_classic(*id))
        .collect();
    println!(
        "cliente: {} | jogo: {} | campeoes: {} (+{} classicos)",
        client_dir.display(),
        game.display(),
        regular.len(),
        champions.len() - regular.len()
    );
    let findings: Vec<client_audit::ChampionFindings> = regular
        .iter()
        .map(|(id, alias)| client_audit::audit_champion(&data, &game, *id, alias))
        .collect();
    let staging = std::env::temp_dir().join("dekan_client_audit_classic");
    if let Err(e) = std::fs::create_dir_all(&staging) {
        println!("pasta temporaria indisponivel: {e}");
        return;
    }
    let classic: Vec<client_audit::ClassicFindings> = champions
        .iter()
        .filter(|(id, _)| client_audit::is_classic(*id))
        .map(|(id, alias)| client_audit::audit_classic(&data, &game, *id, alias, &staging))
        .collect();
    let _ = std::fs::remove_dir_all(&staging); // ignore-ok: probe scratch folder
    let report = format!(
        "{}
{}",
        client_audit::render(&findings),
        client_audit::render_classic(&classic)
    );
    match std::fs::write(&out, report) {
        Ok(()) => println!("relatorio: {}", out.display()),
        Err(e) => println!("relatorio nao gravado em {}: {e}", out.display()),
    }
}

fn run_client_dump(args: &[String]) {
    let Some(client_dir) = args.first().map(PathBuf::from) else {
        println!("uso: client-dump <pasta do cliente> <caminho relativo>");
        return;
    };
    let data = match client_audit::ClientData::open(&client_dir) {
        Ok(data) => data,
        Err(e) => {
            println!("{e}");
            return;
        }
    };
    for relative in &args[1..] {
        match data.read(relative) {
            Some(bytes) => println!("{}", String::from_utf8_lossy(&bytes)),
            None => println!("ausente: {relative}"),
        }
    }
}

fn run_prop_roundtrip(args: &[String]) {
    let root = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|p| args.get(p + 1))
        .map(PathBuf::from);
    let Some(game) = root.or_else(game_dir) else {
        return;
    };
    let final_dir = game.join("DATA").join("FINAL");
    let mut wads: Vec<PathBuf> = ["Champions", "Maps/Shipping"]
        .iter()
        .filter_map(|dir| std::fs::read_dir(final_dir.join(dir)).ok())
        .flat_map(|entries| entries.flatten().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".wad.client"))
        .collect();
    wads.sort();
    let (mut bins, mut objects, mut differs, mut unreadable) = (0usize, 0usize, 0usize, 0usize);
    let mut examples = Vec::new();
    for path in &wads {
        let Ok(wad) = dekan_wad::wad::WadFile::open(path) else {
            unreadable += 1;
            continue;
        };
        let hashes: Vec<u64> = wad.entries().map(|(hash, _)| hash).collect();
        for hash in hashes {
            let Ok(Some(bytes)) = wad.read(hash) else {
                continue;
            };
            if !(bytes.starts_with(b"PROP") || bytes.starts_with(b"PTCH")) {
                continue;
            }
            let Ok(bin) = dekan_wad::prop::parse_prop_file(&bytes) else {
                continue;
            };
            bins += 1;
            for entry in &bin.entries {
                objects += 1;
                let same = dekan_wad::prop::tree::parse_fields(&entry.body)
                    .and_then(|fields| dekan_wad::prop::tree::write_fields(&fields))
                    .is_ok_and(|written| written == entry.body);
                if !same {
                    differs += 1;
                    if examples.len() < 10 {
                        examples.push(format!(
                            "{} entry {hash:016x} object {:08x}",
                            path.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            entry.key_hash
                        ));
                    }
                }
            }
        }
    }
    println!(
        "wads: {} ({unreadable} unreadable) | bins: {bins} | objects: {objects} | not identical: {differs}",
        wads.len()
    );
    for example in &examples {
        println!("  {example}");
    }
    if differs > 0 {
        std::process::exit(1);
    }
}

fn run_skin_audit(args: &[String]) {
    let mut root = None;
    let mut out = skin_audit::default_report_path();
    let mut filters = Vec::new();
    let mut keep: Option<PathBuf> = None;
    let mut options = dekan_classic::generator::GenerationOptions::default();
    let mut forms = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--root" => root = iter.next().map(PathBuf::from),
            "--out" => {
                if let Some(path) = iter.next() {
                    out = PathBuf::from(path);
                }
            }
            "--keep" => keep = iter.next().map(PathBuf::from),
            "--forms" => forms = true,
            "--variant" => match iter.next().map(String::as_str) {
                Some("graph-slot0") => options.graph_in_slot0 = true,
                Some("chroma-classification") => options.chroma_keeps_classification = true,
                other => eprintln!(
                    "unknown variant {other:?}; expected graph-slot0 or chroma-classification"
                ),
            },
            other => filters.push(other.to_owned()),
        }
    }
    let Some(game) = root.or_else(game_dir) else {
        return;
    };
    let aliases = skin_audit::champion_aliases(&game, &filters);
    println!("jogo: {} | campeoes: {}", game.display(), aliases.len());

    let maps = skin_audit::map_hashes(&game);
    println!(
        "WADs de mapa: {:?}",
        maps.iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
    );
    let cache_dir = std::env::temp_dir().join("dekan_skin_audit_cache");
    let staging = keep
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("dekan_skin_audit_mods"));
    let _ = std::fs::remove_dir_all(&staging); // ignore-ok: probe scratch folder may not exist
    if let Err(e) = std::fs::create_dir_all(&staging).and(std::fs::create_dir_all(&cache_dir)) {
        println!("pasta temporaria indisponivel: {e}");
        return;
    }

    let started = std::time::Instant::now();
    let mut reports = Vec::with_capacity(aliases.len());
    for alias in &aliases {
        let report = skin_audit::audit_champion(
            &game,
            alias,
            &maps,
            &cache_dir,
            &staging,
            skin_audit::AuditSettings {
                keep: keep.is_some(),
                options,
                forms,
            },
        );
        let findings = report.skins.values().filter(|f| !f.is_clean()).count()
            + report.forms.values().filter(|f| !f.is_clean()).count();
        println!(
            "{alias}: skins={} formas={} companheiros={:?} achados={findings} varredura_ms={}{}",
            report.skins.len(),
            report.forms.len(),
            report.companion_skins.keys().collect::<Vec<_>>(),
            report.scan_ms,
            report
                .open_error
                .as_deref()
                .map(|e| format!(" ERRO={e}"))
                .unwrap_or_default()
        );
        reports.push(report);
    }
    let missed = skin_audit::missed_companions(&reports, &game);
    let rendered = skin_audit::render(&reports, &missed);
    match std::fs::write(&out, rendered) {
        Ok(()) => println!(
            "
relatorio: {} ({} s)",
            out.display(),
            started.elapsed().as_secs()
        ),
        Err(e) => println!("relatorio nao gravado em {}: {e}", out.display()),
    }
    if keep.is_none() {
        let _ = std::fs::remove_dir_all(&staging); // ignore-ok: probe scratch folder
    }
}

fn run_install_audit(args: &[String]) {
    let Some(phase) = install_audit::parse_phase(args) else {
        eprintln!("usage: cargo xtask install-audit installed | uninstalled [--kept-user-content]");
        std::process::exit(2);
    };

    let registry = install_audit::RegExe;
    let program_dir = install_audit::install_location(&registry).unwrap_or_else(|| {
        std::env::var_os("ProgramW6432")
            .or_else(|| std::env::var_os("ProgramFiles"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
            .join("Dekan")
    });
    let layout = install_audit::Layout {
        program_dir,
        data_dir: dekan_data_dir(),
    };
    let tools_dir = layout.program_dir.join("tools");
    let mut tools = Vec::with_capacity(USER_SUPPLIED_TOOLS.len());
    for (name, constants) in USER_SUPPLIED_TOOLS {
        if !tools_dir.join(name).is_file() {
            println!(
                "  [INFO] {name} not in {} yet (the user supplies it)",
                tools_dir.display()
            );
            continue;
        }
        match constants.first().map(|c| audited_hash(c)) {
            Some(Ok(hash)) => tools.push((name, hash)),
            Some(Err(e)) => {
                eprintln!("[ERRO] {e}");
                std::process::exit(1);
            }
            None => {}
        }
    }
    let sha256 = |path: &std::path::Path| -> Option<String> {
        std::fs::read(path)
            .ok()
            .map(|bytes| dekan_inject::dll_validator::to_hex(&Sha256::digest(&bytes)))
    };
    let mut checks = install_audit::audit_files(&layout, phase, &tools, &sha256);
    checks.extend(install_audit::audit_registry(
        phase,
        &layout.program_dir.join("dekan.exe"),
        &registry,
    ));
    if !install_audit::report(&checks) {
        std::process::exit(1);
    }
}

fn run_client_probe() {
    use dekan_platform::client_window::{
        ClientWindowState, client_window_state, overlay_placement,
    };

    match client_window_state() {
        ClientWindowState::Absent => {
            println!("cliente: AUSENTE (nenhuma janela de classe RCLIENT)");
        }
        ClientWindowState::Hidden => {
            println!("cliente: OCULTO ou MINIMIZADO (rect nao utilizavel; overlay fica escondido)");
        }
        ClientWindowState::Visible(rect) => {
            println!(
                "cliente: VISIVEL em {},{} -> {},{}  ({}x{})",
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                rect.width(),
                rect.height()
            );
            let placement = overlay_placement(rect, 360, 520, 16);
            println!(
                "overlay: {},{} -> {},{}  ({}x{})",
                placement.left,
                placement.top,
                placement.right,
                placement.bottom,
                placement.width(),
                placement.height()
            );
        }
    }
}

fn run_overlay_demo() {
    use dekan_platform::client_window::client_window_state;
    use dekan_platform::overlay_window::{OverlayWindow, track_once};
    use std::time::{Duration, Instant};

    let (overlay, _commands) = match OverlayWindow::spawn() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[ERRO] nao foi possivel criar o overlay: {e}");
            std::process::exit(1);
        }
    };
    let controller = overlay.controller();

    println!("Overlay criado. Acompanhando a janela do cliente por 30s.");
    println!("Restaure o cliente do League se ele estiver minimizado.");

    let started = Instant::now();
    let mut last = String::new();

    while started.elapsed() < Duration::from_secs(30) {
        let placement = track_once(&controller, true);
        let now = match (client_window_state(), placement) {
            (_, Some(rect)) => format!(
                "VISIVEL - overlay em {},{} ({}x{})",
                rect.left,
                rect.top,
                rect.width(),
                rect.height()
            ),
            (state, None) => format!("{state:?} - overlay escondido"),
        };
        if now != last {
            println!("[{:>5.1}s] {now}", started.elapsed().as_secs_f32());
            last = now;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    println!("Fim da demo; overlay encerrado.");
}

fn run_library_probe(champion_id: Option<u32>) {
    use dekan_core::library::{champions_with_content, scan_champion};

    let data = dekan_data_dir();
    let roots = [data.join("library"), data.join("skins")];

    let Some(root) = roots.iter().find(|p| p.is_dir()) else {
        println!("nenhuma biblioteca encontrada em:");
        for r in &roots {
            println!("  {}", r.display());
        }
        return;
    };

    println!("biblioteca: {}", root.display());

    match champion_id {
        Some(id) => {
            let library = scan_champion(root, id);
            println!(
                "campeao {id}: {} skins, {} pacotes instalaveis",
                library.skins.len(),
                library.package_count()
            );
            for skin in library.skins.iter().take(8) {
                println!(
                    "  skin {} ({} chromas){}",
                    skin.id,
                    skin.chromas.len(),
                    if skin.chromas.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " -> {}",
                            skin.chromas
                                .iter()
                                .map(|c| c.id.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                );
            }
            if library.skins.len() > 8 {
                println!("  ... e mais {} skins", library.skins.len() - 8);
            }
        }
        None => {
            let found = champions_with_content(root);
            let total: usize = found.values().sum();
            println!(
                "{} campeoes com conteudo, {total} pacotes instalaveis no total",
                found.len()
            );
        }
    }
}

fn run_catalog_demo(champion_id: u32) {
    use dekan_app::catalog::{catalog_json, load_catalog, resolve_library_root};
    use dekan_platform::client_window::client_window_state;
    use dekan_platform::overlay_window::{OverlayWindow, track_once};
    use std::time::{Duration, Instant};

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("[ERRO] runtime: {e}");
            std::process::exit(1);
        }
    };

    let configured = dekan_data_dir().join("library");
    let root = resolve_library_root(&configured);
    println!("biblioteca: {}", root.display());

    let catalog = runtime.block_on(load_catalog(root, champion_id));
    println!(
        "catalogo: {} ({}) - {} skins, {} entradas",
        catalog.champion_name,
        catalog.champion_id,
        catalog.skins.len(),
        catalog.entry_count()
    );
    for skin in catalog.skins.iter().take(6) {
        println!(
            "  {} [{}]{}",
            skin.name,
            skin.id,
            if skin.chromas.is_empty() {
                String::new()
            } else {
                format!(" + {} chromas", skin.chromas.len())
            }
        );
    }

    let json = match catalog_json(&catalog) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("[ERRO] serializacao: {e}");
            std::process::exit(1);
        }
    };

    let (overlay, mut commands) = match OverlayWindow::spawn() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[ERRO] overlay: {e}");
            std::process::exit(1);
        }
    };
    let controller = overlay.controller();
    controller.set_catalog(json);

    println!("Overlay com o catalogo real. 45s. Restaure o cliente se estiver minimizado.");
    let started = Instant::now();
    let mut last = String::new();
    while started.elapsed() < Duration::from_secs(45) {
        let placement = track_once(&controller, true);
        let now = match (client_window_state(), placement) {
            (_, Some(rect)) => format!("VISIVEL em {},{}", rect.left, rect.top),
            (state, None) => format!("{state:?} - escondido"),
        };
        if now != last {
            println!("[{:>5.1}s] {now}", started.elapsed().as_secs_f32());
            last = now;
        }

        while let Ok(command) = commands.try_recv() {
            match command {
                dekan_core::overlay::OverlayCommand::Select { id } => {
                    match catalog.resolve_target(id) {
                        Some(target) => println!(
                            "  clique -> alvo: campeao {} skin {} chroma {:?} pacote {}",
                            target.champion_id,
                            target.skin_id,
                            target.chroma_id,
                            target.package_entry_id()
                        ),
                        None => println!("  clique -> id {id} nao esta no catalogo (recusado)"),
                    }
                }
                dekan_core::overlay::OverlayCommand::Clear => {
                    println!("  clique -> selecao limpa")
                }
                dekan_core::overlay::OverlayCommand::SetMods { selection } => {
                    println!("  mods -> {selection:?}")
                }
                dekan_core::overlay::OverlayCommand::OpenModsFolder => {
                    println!("  mods -> abrir pasta")
                }
                dekan_core::overlay::OverlayCommand::Random => println!("  dado -> sortear skin"),
                dekan_core::overlay::OverlayCommand::ImportMod { category } => {
                    println!("  mods -> importar em {category:?}")
                }
                dekan_core::overlay::OverlayCommand::ChromaPreview { id } => {
                    println!("  hover -> preview do chroma {id}")
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("Fim da demo.");
}

fn run_ipc_probe(champion_id: u32) {
    use dekan_app::catalog::{catalog_json, load_catalog, resolve_library_root};
    use dekan_core::overlay::OverlayCommand;
    use dekan_platform::overlay_window::OverlayWindow;
    use std::time::{Duration, Instant};

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("[ERRO] runtime: {e}");
            std::process::exit(1);
        }
    };

    let configured = dekan_data_dir().join("library");
    let root = resolve_library_root(&configured);
    let catalog = runtime.block_on(load_catalog(root, champion_id));

    let Some(first_skin) = catalog.skins.first().cloned() else {
        eprintln!("[ERRO] campeao {champion_id} nao tem nenhuma skin na biblioteca local");
        std::process::exit(1);
    };
    let first_chroma = catalog
        .skins
        .iter()
        .find_map(|skin| skin.chromas.first().map(|c| c.id));

    let json = match catalog_json(&catalog) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("[ERRO] serializacao: {e}");
            std::process::exit(1);
        }
    };

    let (overlay, mut commands) = match OverlayWindow::spawn() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[ERRO] overlay: {e}");
            std::process::exit(1);
        }
    };
    let controller = overlay.controller();
    controller.set_catalog(json);
    println!(
        "catalogo carregado: {} ({}) - {} skins",
        catalog.champion_name,
        catalog.champion_id,
        catalog.skins.len()
    );

    std::thread::sleep(Duration::from_millis(600));

    let mut expected: Vec<u32> = vec![first_skin.id];
    controller.eval_script(click_script(first_skin.id));
    if let Some(chroma_id) = first_chroma {
        println!("clicando skin {} e chroma {chroma_id}", first_skin.id);
        expected.push(chroma_id);
        std::thread::sleep(Duration::from_millis(300));
        controller.eval_script(click_script(chroma_id));
    } else {
        println!("clicando skin {} (campeao sem chromas)", first_skin.id);
    }

    let mut received: Vec<u32> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while received.len() < expected.len() && Instant::now() < deadline {
        match commands.try_recv() {
            Ok(OverlayCommand::Select { id }) => {
                match catalog.resolve_target(id) {
                    Some(target) => println!(
                        "  recebido no Rust: id {id} -> campeao {} skin {} chroma {:?} pacote {}",
                        target.champion_id,
                        target.skin_id,
                        target.chroma_id,
                        target.package_entry_id()
                    ),
                    None => println!("  recebido no Rust: id {id} NAO esta no catalogo"),
                }
                received.push(id);
            }
            Ok(OverlayCommand::Clear) => println!("  recebido no Rust: selecao limpa"),
            Ok(OverlayCommand::SetMods { selection }) => {
                println!("  recebido no Rust: mods {selection:?}")
            }
            Ok(OverlayCommand::OpenModsFolder) => {
                println!("  recebido no Rust: abrir pasta de mods")
            }
            Ok(OverlayCommand::Random) => println!("  recebido no Rust: sortear skin"),
            Ok(OverlayCommand::ImportMod { category }) => {
                println!("  recebido no Rust: importar mod em {category:?}")
            }
            Ok(OverlayCommand::ChromaPreview { id }) => {
                println!("  recebido no Rust: preview do chroma {id}")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }

    drop(overlay);

    if received == expected {
        println!("[OK] o clique atravessou a fronteira JS -> Rust: {received:?}");
    } else {
        eprintln!("[FALHA] esperado {expected:?}, recebido {received:?}");
        std::process::exit(1);
    }
}

fn click_script(entry_id: u32) -> String {
    format!(
        "(function() {{ var n = document.querySelector('[data-id=\"{entry_id}\"]');          if (n) {{ n.click(); }} else {{ window.ipc.postMessage(JSON.stringify({{type:'probe-missing'}})); }} }})();"
    )
}

fn run_classic_probe(args: &[String]) {
    use dekan_classic::generator::{ClassicChampion, jade_characters, main_character, slots_for};

    let aliases: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();

    let Some(game) = game_dir() else {
        return;
    };
    let tools = dekan_tools_dir();
    let table = tools.join("hashes.game.txt");
    let cache = std::env::temp_dir().join("dekan_classic_probe_characters.json");
    let known = jade_characters(&table, &cache);
    println!("jogo: {}", game.display());
    println!("personagens jade_* na tabela de hashes: {}", known.len());

    let staging = std::env::temp_dir().join("dekan_classic_probe_mods");
    let _ = std::fs::remove_dir_all(&staging); // ignore-ok: probe scratch folder may not exist
    if let Err(e) = std::fs::create_dir_all(&staging) {
        println!("pasta temporaria indisponivel: {e}");
        return;
    }

    for alias in aliases {
        let alias = alias.as_str();
        println!("\n== {alias}");
        let champion = match ClassicChampion::open(&game, alias) {
            Ok(champion) => champion,
            Err(e) => {
                println!("  nao abriu: {e}");
                continue;
            }
        };
        let regular = alias.to_ascii_lowercase();
        let jade = main_character(alias);
        let regular_numbers = champion.skin_numbers(&regular, 1000);
        let jade_numbers = champion.skin_numbers(&jade, 1000);
        let present = champion.present_characters(&known);
        println!("  personagens Classic presentes: {present:?}");

        let started = std::time::Instant::now();
        let from_bins = champion.jade_names_in_bins();
        let present_from_bins = champion.present_characters(&from_bins);
        println!(
            "  pelos .bin (sem tabela, {} ms): nomes {:?} -> presentes {:?} {}",
            started.elapsed().as_millis(),
            from_bins,
            present_from_bins,
            if present_from_bins == present {
                "IGUAL"
            } else {
                "DIFERENTE"
            }
        );

        println!(
            "  {regular}: {} numeros {:?}",
            regular_numbers.len(),
            regular_numbers
        );
        println!(
            "  {jade}: {} numeros {:?}",
            jade_numbers.len(),
            jade_numbers
        );
        let only_regular: Vec<u32> = regular_numbers
            .iter()
            .copied()
            .filter(|n| !jade_numbers.contains(n))
            .collect();
        println!("  existem no normal e NAO no Classic: {only_regular:?}");

        let mut built = 0usize;
        let mut failed = Vec::new();
        for number in jade_numbers.iter().copied().filter(|n| *n != 0) {
            match champion.build_mod(number, &slots_for(None), &known, &staging) {
                Ok(_) => built += 1,
                Err(e) => failed.push(format!("{number}: {e}")),
            }
        }
        println!(
            "  mods Classic gerados dos bins reais: {built} ok, {} falharam {failed:?}",
            failed.len()
        );
    }
    let _ = std::fs::remove_dir_all(&staging); // ignore-ok: probe scratch folder
}

#[derive(Default)]
struct WadTypeStats {
    entries: usize,
    decoded: usize,
    failed: usize,
    size_mismatch: usize,
    checksum_mismatch: usize,
    zstd_magic_first: usize,
    gzip_magic_first: usize,
    compressed_bytes: u64,
}

fn run_wad_probe(args: &[String]) {
    use std::collections::BTreeMap;

    use dekan_wad::hash::content_checksum;
    use dekan_wad::wad::WadArchive;

    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
    const GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];
    const MAX_REPORTED: usize = 10;

    let mut root = None;
    let mut filters = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--root" {
            root = iter.next().map(PathBuf::from);
        } else {
            filters.push(arg.to_ascii_lowercase());
        }
    }
    if filters.is_empty() {
        println!(
            "uso: cargo xtask wad-probe [--root <pasta>] <trecho do caminho...>   (ex.: Champions/Annie. Maps/Shipping; '.' = todos)"
        );
        return;
    }
    let final_dir = match root {
        Some(root) => root,
        None => {
            let Some(game) = game_dir() else {
                return;
            };
            game.join("DATA").join("FINAL")
        }
    };
    let mut files = Vec::new();
    collect_wad_files(&final_dir, &mut files);
    files.retain(|p| {
        let rel = p
            .strip_prefix(&final_dir)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        filters.iter().any(|f| rel.contains(f.as_str()))
    });
    files.sort();
    println!(
        "raiz: {}  WADs selecionados: {}",
        final_dir.display(),
        files.len()
    );

    let mut stats: BTreeMap<u8, WadTypeStats> = BTreeMap::new();
    let mut reported = Vec::new();
    for file in &files {
        let data = match std::fs::read(file) {
            Ok(data) => data,
            Err(e) => {
                println!("  {} nao leu: {e}", file.display());
                continue;
            }
        };
        let mut archive = match WadArchive::parse(&data) {
            Ok(archive) => archive,
            Err(e) => {
                println!("  {} nao abriu: {e}", file.display());
                continue;
            }
        };

        if let Some(name) = dekan_wad::wad::subchunk_toc_name(file) {
            archive.load_subchunk_toc(&name);
        }
        let mut file_problems = 0usize;
        for entry in archive.entries() {
            let kind = entry.compression as u8;
            let s = stats.entry(kind).or_default();
            s.entries += 1;
            s.compressed_bytes += entry.compressed_size as u64;
            let payload = data
                .get(entry.offset..entry.offset + entry.compressed_size)
                .unwrap_or(&[]);
            if payload.starts_with(&ZSTD_MAGIC) {
                s.zstd_magic_first += 1;
            }
            if payload.starts_with(&GZIP_MAGIC) {
                s.gzip_magic_first += 1;
            }
            if content_checksum(payload) != entry.checksum {
                s.checksum_mismatch += 1;
                file_problems += 1;
            }
            match archive.read_entry(entry) {
                Ok(_) => s.decoded += 1,
                Err(e) => {
                    file_problems += 1;
                    if matches!(e, dekan_wad::error::WadError::SizeMismatch { .. }) {
                        s.size_mismatch += 1;
                    } else {
                        s.failed += 1;
                    }
                    if reported.len() < MAX_REPORTED {
                        let head: Vec<String> =
                            payload.iter().take(8).map(|b| format!("{b:02x}")).collect();
                        reported.push(format!(
                            "{} {:016x} tipo {kind}: {e} (inicio {})",
                            file.display(),
                            entry.path_hash,
                            head.join(" ")
                        ));
                    }
                }
            }
        }

        println!(
            "  {:>6} entradas  {:>4} problemas  {}",
            archive.entries().len(),
            file_problems,
            file.strip_prefix(&final_dir).unwrap_or(file).display()
        );
    }

    println!(
        "\ntipo  entradas  decodificadas  falhas  tamanho!=TOC  checksum!=TOC  comeca_zstd  comeca_gzip  MB_comprimidos"
    );
    for (kind, s) in &stats {
        println!(
            "{kind:>4}  {:>8}  {:>13}  {:>6}  {:>12}  {:>13}  {:>11}  {:>11}  {:>14.1}",
            s.entries,
            s.decoded,
            s.failed,
            s.size_mismatch,
            s.checksum_mismatch,
            s.zstd_magic_first,
            s.gzip_magic_first,
            s.compressed_bytes as f64 / 1_048_576.0
        );
    }
    if !reported.is_empty() {
        println!("\nprimeiros problemas:");
        for line in &reported {
            println!("  {line}");
        }
    }
}

fn run_wad_types(args: &[String]) {
    use std::collections::BTreeMap;

    let root = match args.iter().position(|a| a == "--root") {
        Some(i) => match args.get(i + 1) {
            Some(root) => PathBuf::from(root),
            None => {
                println!("uso: cargo xtask wad-types [--root <pasta>]");
                return;
            }
        },
        None => match game_dir() {
            Some(game) => game.join("DATA").join("FINAL"),
            None => return,
        },
    };
    let mut files = Vec::new();
    collect_wad_files(&root, &mut files);
    println!("raiz: {} | WADs: {}", root.display(), files.len());

    let mut by_type: BTreeMap<u8, usize> = BTreeMap::new();
    let mut samples: BTreeMap<u8, Vec<String>> = BTreeMap::new();
    let mut unreadable = 0usize;
    for file in &files {
        let wad = match dekan_wad::wad::WadFile::open(file) {
            Ok(wad) => wad,
            Err(e) => {
                unreadable += 1;
                println!("  nao abriu {}: {e}", file.display());
                continue;
            }
        };
        for entry in wad.toc() {
            let kind = entry.compression as u8;
            *by_type.entry(kind).or_default() += 1;
            if matches!(kind, 1 | 2) && samples.get(&kind).is_none_or(|s| s.len() < 5) {
                let head = match wad.read_raw(entry) {
                    Ok(raw) => raw
                        .iter()
                        .take(24)
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                    Err(e) => format!("ilegivel: {e}"),
                };
                samples.entry(kind).or_default().push(format!(
                    "{:016x} em {}: {head}",
                    entry.path_hash,
                    file.strip_prefix(&root).unwrap_or(file).display()
                ));
            }
        }
    }

    println!("\ntipo  entradas   (como o dekan-wad le o numero)");
    for (kind, count) in &by_type {
        let name = dekan_wad::wad::CompressionType::from_type_byte(*kind)
            .map(|c| format!("{c:?}"))
            .unwrap_or_else(|_| "?".into());
        println!("  {kind}   {count:>9}   {name}");
    }
    for (kind, lines) in &samples {
        println!("\namostras do tipo {kind}:");
        for line in lines {
            println!("  {line}");
        }
    }
    if unreadable > 0 {
        println!("\nWADs ilegiveis: {unreadable}");
    }
}

fn collect_wad_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_wad_files(&path, out);
        } else if path.to_string_lossy().ends_with(".wad.client") {
            out.push(path);
        }
    }
}

fn run_wad_writer_probe(args: &[String]) {
    use dekan_wad::hash::content_checksum;
    use dekan_wad::wad::WadFile;
    use dekan_wad::writer::{WadWriter, WriterEntry};
    use std::time::Instant;

    let mut root = None;
    let mut filters = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--root" {
            root = iter.next().map(PathBuf::from);
        } else {
            filters.push(arg.to_ascii_lowercase());
        }
    }
    if filters.is_empty() {
        println!(
            "uso: cargo xtask wad-writer-probe [--root <pasta>] <filtro...>   (ex.: Champions/Annie, Maps/Shipping; '.' = todos)"
        );
        return;
    }
    let scratch =
        std::env::temp_dir().join(format!("dekan_wad_writer_probe_{}", std::process::id()));
    let final_dir = match root {
        Some(root) => root,
        None => {
            let Some(game) = game_dir() else {
                return;
            };
            game.join("DATA").join("FINAL")
        }
    };
    let mut files = Vec::new();
    collect_wad_files(&final_dir, &mut files);
    files.retain(|p| {
        let rel = p
            .strip_prefix(&final_dir)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        filters.iter().any(|f| rel.contains(f.as_str()))
    });
    files.sort();
    println!(
        "raiz: {} | WADs selecionados: {}",
        final_dir.display(),
        files.len()
    );

    let (mut total_entries, mut total_bad) = (0usize, 0usize);
    for file in &files {
        let rel = file.strip_prefix(&final_dir).unwrap_or(file);
        let source = match WadFile::open_toc_only(file) {
            Ok(source) => source,
            Err(e) => {
                println!("  {} nao abriu: {e}", rel.display());
                continue;
            }
        };
        let mut writer = WadWriter::rebased_on(&source);
        let index = writer.add_source(file);
        for entry in source.toc() {
            writer.insert(entry.path_hash, WriterEntry::from_wad(index, entry));
        }
        let out = scratch.join(rel);
        let started = Instant::now();
        let outcome = match writer.write_to_file(&out, &|| false) {
            Ok(outcome) => outcome,
            Err(e) => {
                println!("  {} copia falhou: {e}", rel.display());
                continue;
            }
        };
        let elapsed = started.elapsed();

        let copy = match WadFile::open_toc_only(&out) {
            Ok(copy) => copy,
            Err(e) => {
                println!("  [ERRO] {} copia ilegivel: {e}", rel.display());
                total_bad += 1;
                continue;
            }
        };
        let mut bad = 0usize;
        if copy.len() != source.len() || copy.signature() != source.signature() {
            bad += 1;
        }
        for original in source.toc() {
            let same = copy.entry(original.path_hash).is_some_and(|copied| {
                copied.compression == original.compression
                    && copied.compressed_size == original.compressed_size
                    && copied.uncompressed_size == original.uncompressed_size
                    && copied.checksum == original.checksum
                    && copied.subchunk_count == original.subchunk_count
                    && copied.first_subchunk == original.first_subchunk
                    && copy
                        .read_raw(copied)
                        .is_ok_and(|raw| content_checksum(&raw) == original.checksum)
            });
            if !same {
                bad += 1;
            }
        }
        total_entries += source.len();
        total_bad += bad;
        println!(
            "  {:>6} entradas | {:>8.2} MB | copia {:>6} ms | {} divergentes | {}",
            source.len(),
            outcome.bytes() as f64 / 1_048_576.0,
            elapsed.as_millis(),
            bad,
            rel.display()
        );
        let _ = std::fs::remove_file(&out); // ignore-ok: probe scratch copy
    }
    let _ = std::fs::remove_dir_all(&scratch); // ignore-ok: probe scratch folder
    println!("\ntotal: {total_entries} entradas, {total_bad} divergentes");
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
