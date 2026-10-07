use super::*;

fn compare(limit: u32) -> Vec<u8> {
    let mut code = vec![0x3d];
    code.extend_from_slice(&limit.to_le_bytes());
    code.extend_from_slice(&[0x0f, 0x86, 0xd5, 0x00, 0x00, 0x00]);
    code
}

#[test]
fn the_build_limit_is_the_game_build_check_inside_the_dll() {
    let mut dll = vec![0x90; 64];
    dll.extend(compare(0x6ad4_6e70));
    dll.extend([0xcc; 32]);
    assert_eq!(dll_build_limit(&dll), Some(0x6ad4_6e70));
}

#[test]
fn the_1_26_and_1_27_builds_have_the_limits_their_releases_state() {
    let old = [
        0x89, 0xc2, 0x84, 0xc0, 0x0f, 0x85, 0xd6, 0xfc, 0xff, 0xff, 0xe9, 0x76, 0x03, 0x00, 0x00,
    ]
    .into_iter()
    .chain(compare(0x6ac1_f970))
    .collect::<Vec<u8>>();
    assert_eq!(
        dll_build_limit(&old),
        Some(0x6ac1_f970),
        "2026-10-04 07:00 UTC"
    );
    assert_eq!(
        dll_build_limit(&compare(0x6ad4_6e70)),
        Some(0x6ad4_6e70),
        "2026-10-18 07:00 UTC"
    );
}

#[test]
fn an_unknown_or_ambiguous_limit_is_not_guessed() {
    assert_eq!(dll_build_limit(b"no compare here"), None);
    assert_eq!(
        dll_build_limit(&compare(1_234_567)),
        None,
        "not a plausible date"
    );
    assert_eq!(
        dll_build_limit(&compare(0x6ad4_6e71)),
        None,
        "not on the hour"
    );
    let two: Vec<u8> = compare(0x6ad4_6e70)
        .into_iter()
        .chain(compare(0x6b73_2f10))
        .collect();
    assert_eq!(dll_build_limit(&two), None, "two different limits");
    let same: Vec<u8> = compare(0x6ad4_6e70)
        .into_iter()
        .chain(compare(0x6ad4_6e70))
        .collect();
    assert_eq!(dll_build_limit(&same), Some(0x6ad4_6e70));
}

#[test]
fn an_unsigned_injector_file_is_refused_with_its_path() {
    let path = std::env::temp_dir().join(format!("dekan_trust_{}.dll", std::process::id()));
    std::fs::write(&path, b"MZ unsigned").expect("fixture");
    match verify_injector_file(&path) {
        Err(InjectError::UntrustedInjector { path: shown, .. }) => {
            assert!(shown.contains("dekan_trust_"))
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    std::fs::remove_file(path).ok();
}

#[test]
#[ignore = "needs the LTK injector in Dekan's tools folder"]
fn the_installed_injector_is_trusted_and_names_its_limit() {
    let tools = Path::new(r"C:\Program Files\Dekan\tools");
    verify_injector_file(&tools.join("ltk_patcher_host.exe")).expect("host trusted");
    verify_injector_file(&tools.join("ltk_patcher_dll.dll")).expect("dll trusted");
    let dll = std::fs::read(tools.join("ltk_patcher_dll.dll")).expect("dll");
    assert!(dll_build_limit(&dll).is_some());
}
