use ring::signature::{Ed25519KeyPair, KeyPair};

use super::*;

fn v(tag: &str) -> Version {
    Version::parse(tag).expect("version")
}

fn key_pair() -> Ed25519KeyPair {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).expect("pkcs8");
    Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("key pair")
}

fn asset(name: &str, url: &str) -> ReleaseAsset {
    ReleaseAsset {
        name: name.to_owned(),
        browser_download_url: url.to_owned(),
    }
}

#[test]
fn test_the_setup_name_matches_the_release_asset() {
    assert_eq!(setup_name(v("v1.3")), "Dekan-Setup-1.3-x64.exe");
    assert_eq!(setup_name(v("2.0")), "Dekan-Setup-2.0-x64.exe");
}

#[test]
fn test_the_hash_is_read_from_the_signed_list() {
    let hash = "AB".repeat(32);
    let sums = format!("{hash} *Dekan-Setup-1.3-x64.exe");
    assert_eq!(
        expected_hash(&sums, "Dekan-Setup-1.3-x64.exe"),
        Some(hash.to_ascii_lowercase())
    );
    let text_mode = format!(
        "other  x.exe\n{}  Dekan-Setup-1.3-x64.exe\n",
        "0".repeat(64)
    );
    assert_eq!(
        expected_hash(&text_mode, "Dekan-Setup-1.3-x64.exe"),
        Some("0".repeat(64))
    );
    assert_eq!(expected_hash(&sums, "Dekan-Setup-1.2-x64.exe"), None);
    assert_eq!(
        expected_hash("abc *Dekan-Setup-1.3-x64.exe", "Dekan-Setup-1.3-x64.exe"),
        None
    );
}

#[test]
fn test_hex_round_trips_and_refuses_garbage() {
    let bytes = [0u8, 1, 0xab, 0xff];
    assert_eq!(encode_hex(&bytes), "0001abff");
    assert_eq!(decode_hex("0001ABff"), Some(bytes.to_vec()));
    for bad in ["0", "zz", "0g", "é1"] {
        assert_eq!(decode_hex(bad), None, "{bad}");
    }
}

#[test]
fn test_only_the_release_key_signature_is_accepted() {
    let pair = key_pair();
    let public = pair.public_key().as_ref();
    let sums = b"00 *Dekan-Setup-1.3-x64.exe";
    let signature = encode_hex(pair.sign(sums).as_ref());
    assert_eq!(verify_sums_with(public, sums, signature.as_bytes()), Ok(()));
    assert_eq!(
        verify_sums_with(public, sums, format!("{signature}\r\n").as_bytes()),
        Ok(())
    );
    assert!(
        verify_sums_with(public, b"11 *Dekan-Setup-1.3-x64.exe", signature.as_bytes()).is_err()
    );
    assert!(
        verify_sums_with(key_pair().public_key().as_ref(), sums, signature.as_bytes()).is_err()
    );
    assert!(verify_sums_with(public, sums, b"not hex").is_err());
}

#[test]
fn test_assets_must_come_from_the_project_releases() {
    let repo = REPOSITORY.trim_end_matches('/');
    let offer = Offer {
        version: v("1.3"),
        assets: vec![
            asset(
                SUMS_FILE,
                &format!("{repo}/releases/download/v1.3/SHA256SUMS"),
            ),
            asset(SIGNATURE_FILE, "https://example.com/SHA256SUMS.sig"),
        ],
    };
    assert_eq!(
        asset_url(&offer, SUMS_FILE),
        Ok(format!("{repo}/releases/download/v1.3/SHA256SUMS").as_str())
    );
    assert!(asset_url(&offer, SIGNATURE_FILE).is_err());
    assert!(asset_url(&offer, &setup_name(v("1.3"))).is_err());
}

#[test]
fn test_a_pending_update_is_taken_once() {
    let pending = PendingUpdate::default();
    assert_eq!(pending.take(), None);
    let staged = StagedSetup {
        path: PathBuf::from("setup.exe"),
        sha256: "0".repeat(64),
    };
    pending.clone().set(staged.clone()).expect("set");
    assert_eq!(pending.take(), Some(staged));
    assert_eq!(pending.take(), None);
}

#[test]
fn test_only_an_installed_copy_updates_itself() {
    let dir = std::env::temp_dir().join(format!("dekan_self_update_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let exe = dir.join("dekan.exe");
    assert!(!is_installed_copy(&exe));
    std::fs::write(dir.join(UNINSTALLER), b"").expect("uninstaller");
    assert!(is_installed_copy(&exe));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_the_installer_is_hashed_again_and_held_before_it_runs() {
    let dir = std::env::temp_dir().join(format!("dekan_self_update_hold_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("setup.exe");
    std::fs::write(&path, b"setup").expect("setup");
    let staged = StagedSetup {
        path: path.clone(),
        sha256: encode_hex(&Sha256::digest(b"setup")),
    };
    let held = open_verified(&staged).expect("verified");
    assert!(std::fs::write(&path, b"swapped").is_err());
    assert!(std::fs::remove_file(&path).is_err());
    drop(held);
    std::fs::write(&path, b"swapped").expect("rewrite");
    assert!(open_verified(&staged).is_err());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn test_dekan_closes_for_an_update_only_outside_queue_and_matches() {
    for phase in [
        GamePhase::None,
        GamePhase::WaitingForStats,
        GamePhase::EndOfGame,
    ] {
        assert!(can_install(phase), "{phase:?}");
    }
    for phase in [
        GamePhase::Lobby,
        GamePhase::Matchmaking,
        GamePhase::ReadyCheck,
        GamePhase::ChampSelect,
        GamePhase::Finalization,
        GamePhase::GameStart,
        GamePhase::InProgress,
        GamePhase::Reconnect,
    ] {
        assert!(!can_install(phase), "{phase:?}");
    }
}
