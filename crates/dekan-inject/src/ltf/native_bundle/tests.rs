use super::*;

const LAUNCHER_BYTES: &[u8] = include_bytes!("../../../../../vendor/native/dekan_launcher.exe");
const BRIDGE_BYTES: &[u8] = include_bytes!("../../../../../vendor/native/dekan_bridge.dll");

#[test]
fn supplied_payloads_match_the_release_and_a_changed_byte_is_refused() {
    verify_bytes(LAUNCHER, LAUNCHER_BYTES).expect("supplied launcher");
    verify_bytes(COMPAT_LAUNCHER, LAUNCHER_BYTES).expect("unmodified shim compatibility alias");
    verify_bytes(BRIDGE, BRIDGE_BYTES).expect("supplied bridge");
    verify_bytes(COMPAT_BRIDGE, BRIDGE_BYTES).expect("unmodified compatibility alias");
    let mut changed = BRIDGE_BYTES.to_vec();
    changed[100] ^= 1;
    assert!(verify_bytes(BRIDGE, &changed).is_err());
    assert!(verify_bytes("other.dll", BRIDGE_BYTES).is_err());
}

#[test]
fn packaging_creates_a_matching_alias_and_rejects_stale_or_missing_tools() {
    let root = std::env::temp_dir().join(format!("dekan_native_bundle_{}", std::process::id()));
    let source = root.join("source");
    let dest = root.join("tools");
    std::fs::create_dir_all(&source).expect("source");
    std::fs::write(source.join(LAUNCHER), LAUNCHER_BYTES).expect("launcher");
    std::fs::write(source.join(BRIDGE), BRIDGE_BYTES).expect("bridge");
    copy_bundle(&source, &dest).expect("copy");
    assert_eq!(
        std::fs::read(dest.join(COMPAT_BRIDGE)).expect("alias"),
        BRIDGE_BYTES
    );
    assert_eq!(
        std::fs::read(dest.join(COMPAT_LAUNCHER)).expect("shim alias"),
        LAUNCHER_BYTES
    );
    verify_directory(&dest).expect("complete current bundle");
    std::fs::write(dest.join(COMPAT_LAUNCHER), b"old launcher").expect("stale shim alias");
    assert!(verify_directory(&dest).is_err());
    copy_bundle(&source, &dest).expect("repair shim alias");
    std::fs::write(dest.join(COMPAT_BRIDGE), b"old bridge").expect("stale alias");
    assert!(verify_directory(&dest).is_err());
    copy_bundle(&source, &dest).expect("repair stale alias");
    std::fs::remove_file(dest.join(COMPAT_BRIDGE)).expect("remove alias");
    assert!(verify_directory(&dest).is_err());
    std::fs::write(source.join(LAUNCHER), b"old launcher").expect("stale source");
    assert!(copy_bundle(&source, &dest).is_err());
    let _ = std::fs::remove_dir_all(root); // ignore-ok: fixture cleanup
}
