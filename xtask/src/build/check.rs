use crate::adr008::run_adr008_check;
use crate::lint::file_size::run_file_size_check;
use crate::{check_status, comments};
use std::process::Command;

pub(crate) fn run_check() {
    println!(
        "==> Step 0/4: Error-handling sweep (every discarded Result justified), no comments in code and no code file over 400 lines..."
    );
    run_adr008_check();
    comments::run(&[]);
    run_file_size_check();

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
