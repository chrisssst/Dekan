use super::*;

#[test]
fn test_parse_valid_lockfile() {
    let lf = Lockfile::parse("LeagueClient:12345:55667:supersecrettoken:https\n").unwrap();
    assert_eq!(
        (lf.process_name.as_str(), lf.pid, lf.port),
        ("LeagueClient", 12345, 55667)
    );
    assert_eq!(
        (lf.auth_token.as_str(), lf.protocol.as_str()),
        ("supersecrettoken", "https")
    );
    assert_eq!(
        (lf.base_url(), lf.ws_url()),
        (
            "https://127.0.0.1:55667".into(),
            "wss://127.0.0.1:55667".into()
        )
    );
    let raw_b64 = &lf.basic_auth_header()["Basic ".len()..];
    assert_eq!(
        String::from_utf8(BASE64.decode(raw_b64).unwrap()).unwrap(),
        "riot:supersecrettoken"
    );
}

#[test]
fn test_rejects_malformed_lockfiles() {
    assert!(
        matches!(Lockfile::parse("LeagueClient:12345:55667:token").unwrap_err(), LcuError::InvalidLockfile(m) if m.contains("expected 5"))
    );
    assert!(
        matches!(Lockfile::parse("LeagueClient:12345:notaport:token:https").unwrap_err(), LcuError::InvalidLockfile(m) if m.contains("invalid port"))
    );
    assert!(
        matches!(Lockfile::parse("LeagueClient:12345:55667::https").unwrap_err(), LcuError::InvalidLockfile(m) if m.contains("token is empty"))
    );
}

#[test]
fn test_parse_ux_cmdline_regional() {
    let cmd = r#""D:\WeGame\LeagueClientUx.exe" --app-port=54321 --remoting-auth-token="cn-token_99" --region=CN"#;
    let lf = Lockfile::parse_ux_cmdline(cmd, 8888).expect("regional cmdline");
    assert_eq!(
        (
            lf.pid,
            lf.port,
            lf.auth_token.as_str(),
            lf.protocol.as_str()
        ),
        (8888, 54321, "cn-token_99", "https")
    );
    assert!(Lockfile::parse_ux_cmdline("--app-port=0 --remoting-auth-token=x", 1).is_none());
}

#[test]
fn test_find_lockfile_path_explicit() {
    let dir = std::env::temp_dir().join(format!("dekan_test_lock_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir); // ignore-ok: test temp dir
    let lock_path = dir.join("lockfile");
    let live_pid = std::process::id();
    std::fs::write(
        &lock_path,
        format!("LeagueClient:{live_pid}:456:secret:https"),
    )
    .unwrap();
    assert_eq!(
        Lockfile::find_lockfile_path(Some(&lock_path)),
        Some(lock_path.clone())
    );
    let lock = Lockfile::discover(Some(&lock_path)).unwrap();
    assert_eq!(
        (lock.pid, lock.port, lock.auth_token.as_str()),
        (live_pid, 456, "secret")
    );
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: test temp dir teardown
}
