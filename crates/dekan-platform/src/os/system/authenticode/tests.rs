use super::*;

fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("dekan_sig_{}_{name}", std::process::id()));
    std::fs::write(&path, bytes).expect("fixture");
    path
}

#[test]
fn an_unsigned_file_is_untrusted() {
    let path = temp("plain.exe", b"MZ not really a program");
    assert!(matches!(
        signer(&path),
        Err(SignatureError::Untrusted { .. })
    ));
    std::fs::remove_file(path).ok();
}

#[test]
fn a_missing_file_is_untrusted_not_a_panic() {
    let path = std::env::temp_dir().join("dekan_sig_does_not_exist.dll");
    assert!(matches!(
        signer(&path),
        Err(SignatureError::Untrusted { .. })
    ));
}

#[test]
#[ignore = "needs a signed LTK injector in Dekan's tools folder"]
fn the_installed_injector_is_signed_by_its_publisher_and_a_flipped_byte_breaks_it() {
    let dll = std::path::Path::new(r"C:\Program Files\Dekan\tools\ltk_patcher_dll.dll");
    assert_eq!(signer(dll).expect("signed"), "Natoken LLC");
    let mut bytes = std::fs::read(dll).expect("read");
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    let tampered = temp("tampered.dll", &bytes);
    assert!(matches!(
        signer(&tampered),
        Err(SignatureError::Untrusted { .. })
    ));
    std::fs::remove_file(tampered).ok();
}
