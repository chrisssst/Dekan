use crate::check_status;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

pub(crate) fn release_rustflags(workspace_root: &std::path::Path) -> String {
    let mut flags: Vec<String> = vec![
        "-C".into(),
        "target-feature=+crt-static".into(),
        "-C".into(),
        "target-cpu=x86-64-v2".into(),
    ];
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

pub(crate) fn run_package() {
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
                let hash_relay = dekan_inject::dll_validator::to_hex(&Sha256::digest(&bytes_relay));
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

pub(crate) const USER_SUPPLIED_TOOLS: [&str; 2] = [
    dekan_inject::ltk_host::HOST_EXE,
    dekan_inject::ltk_host::DLL_FILE,
];

pub(crate) fn clean_dist(dist_dir: &std::path::Path) {
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

pub(crate) fn run_installer(args: &[String]) {
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

#[cfg(test)]
#[path = "package_tests.rs"]
mod tests;
