use std::io::Write;
use std::path::Path;

use dekan_app::self_update::{SIGNATURE_FILE, decode_hex, encode_hex, verify_sums};
use ring::signature::{Ed25519KeyPair, KeyPair};

const KEY_ENV: &str = "RELEASE_SIGNING_KEY";

fn fail(message: &str) -> ! {
    eprintln!("[ERROR] {message}");
    std::process::exit(1);
}

pub(crate) fn run_release_key(args: &[String]) {
    let Some(out) = args.first().map(Path::new) else {
        eprintln!("usage: cargo xtask release-key <private key file outside the repository>");
        std::process::exit(2);
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .unwrap_or_else(|e| fail(&format!("repository root: {e}")));
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .unwrap_or_else(|e| fail(&format!("{}: {e}", out.display())));
    if parent.starts_with(&repo) {
        fail("the private key must live outside the repository");
    }
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
        .unwrap_or_else(|_| fail("the key pair could not be generated"));
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .unwrap_or_else(|_| fail("the generated key could not be read back"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .unwrap_or_else(|e| fail(&format!("{}: {e}", out.display())));
    file.write_all(encode_hex(pkcs8.as_ref()).as_bytes())
        .unwrap_or_else(|e| fail(&format!("{}: {e}", out.display())));
    let public: Vec<String> = pair
        .public_key()
        .as_ref()
        .iter()
        .map(|b| format!("0x{b:02x}"))
        .collect();
    println!("[OK] Private key written to {}", out.display());
    println!("Store its content as the {KEY_ENV} repository secret, then embed the public key:");
    println!(
        "pub const RELEASE_PUBLIC_KEY: [u8; 32] = [{}];",
        public.join(", ")
    );
}

pub(crate) fn run_sign_sums(args: &[String]) {
    let Some(sums_path) = args.first().map(Path::new) else {
        eprintln!("usage: cargo xtask sign-sums <SHA256SUMS> (key in {KEY_ENV})");
        std::process::exit(2);
    };
    let key = std::env::var(KEY_ENV).unwrap_or_else(|_| fail(&format!("{KEY_ENV} is not set")));
    let pkcs8 = decode_hex(key.trim()).unwrap_or_else(|| fail(&format!("{KEY_ENV} is not hex")));
    let pair = Ed25519KeyPair::from_pkcs8(&pkcs8)
        .unwrap_or_else(|_| fail(&format!("{KEY_ENV} is not an Ed25519 PKCS#8 key")));
    let sums =
        std::fs::read(sums_path).unwrap_or_else(|e| fail(&format!("{}: {e}", sums_path.display())));
    let signature = encode_hex(pair.sign(&sums).as_ref());
    if let Err(e) = verify_sums(&sums, signature.as_bytes()) {
        fail(&format!(
            "{e}: {KEY_ENV} is not the key whose public half dekan-app embeds"
        ));
    }
    let out = sums_path.with_file_name(SIGNATURE_FILE);
    std::fs::write(&out, &signature).unwrap_or_else(|e| fail(&format!("{}: {e}", out.display())));
    println!("[OK] {} signed into {}", sums_path.display(), out.display());
}
