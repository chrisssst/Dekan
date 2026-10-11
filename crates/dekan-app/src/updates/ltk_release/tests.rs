use super::*;

const DLL_A: &str = "6d419057e6667994ba752ad0fb089b363db98267618644d7f7b6632441a21d74";
const DLL_B: &str = "07a43bf36a389eb00f6276e333bd7f2b95218f25a58e1e128ff4d2e4ab2dc99b";

fn list(versions: &[&str]) -> Vec<String> {
    versions.iter().map(|v| (*v).to_owned()).collect()
}

fn trusted(dll: &str) -> Injector {
    Injector::Trusted {
        dll_sha256: dll.to_owned(),
    }
}

#[test]
fn published_versions_come_newest_first_without_drafts_or_prereleases() {
    let body = r#"[
        {"tag_name":"v1.24.0"},
        {"tag_name":"v1.26.1","draft":false,"prerelease":false},
        {"tag_name":"v1.27.0","prerelease":true},
        {"tag_name":"v1.28.0","draft":true},
        {"tag_name":"nightly"},
        {"tag_name":"v1.26.0"}
    ]"#;
    assert_eq!(
        published_versions(body),
        Ok(list(&["1.26.1", "1.26.0", "1.24.0"]))
    );
    assert!(published_versions("not json").is_err());
}

#[test]
fn the_compatible_version_is_the_newest_release_signed_by_the_publisher() {
    let mut verdicts = Verdicts::default();
    verdicts.insert("1.28.0", Injector::Untrusted);
    verdicts.insert("1.27.0", trusted(DLL_A));
    verdicts.insert("1.26.1", trusted(DLL_B));
    let status = verdicts
        .status(&list(&["1.28.0", "1.27.0", "1.26.1"]))
        .expect("status");
    assert_eq!(status.latest, "1.28.0");
    assert!(!status.latest_trusted);
    assert_eq!(status.compatible.as_deref(), Some("1.27.0"));
    assert_eq!(status.compatible_dll.as_deref(), Some(DLL_A));
    assert_eq!(verdicts.status(&list(&["9.9.9"])), None);
    assert_eq!(verdicts.status(&[]), None);
}

#[test]
fn an_update_is_offered_only_when_the_installed_dll_differs() {
    let status = LtkStatus {
        latest: "1.27.0".into(),
        latest_trusted: true,
        compatible: Some("1.27.0".into()),
        compatible_dll: Some(DLL_A.into()),
    };
    assert_eq!(status.offers_update_over(Some(DLL_B)), Some("1.27.0"));
    assert_eq!(
        status.offers_update_over(None),
        Some("1.27.0"),
        "missing files"
    );
    assert_eq!(status.offers_update_over(Some(DLL_A)), None);
    assert_eq!(
        status.offers_update_over(Some(&DLL_A.to_ascii_uppercase())),
        None
    );
    let nothing_trusted = LtkStatus {
        compatible: None,
        compatible_dll: None,
        ..status
    };
    assert_eq!(nothing_trusted.offers_update_over(Some(DLL_B)), None);
}

#[test]
fn the_cache_orders_versions_numerically() {
    let mut verdicts = Verdicts::default();
    verdicts.insert("1.9.0", trusted(DLL_B));
    verdicts.insert("1.10.0", Injector::Untrusted);
    let status = verdicts.status_from_cache().expect("status");
    assert_eq!(status.latest, "1.10.0");
    assert_eq!(status.compatible.as_deref(), Some("1.9.0"));
}

#[test]
fn verdicts_round_trip_and_drop_what_they_cannot_read() {
    let dir = std::env::temp_dir().join(format!("dekan_ltk_check_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("fixture dir");
    assert_eq!(load_verdicts(&dir), Verdicts::default());

    let mut verdicts = Verdicts::default();
    verdicts.insert("1.27.0", trusted(DLL_A));
    verdicts.insert("1.28.0", Injector::Untrusted);
    save_verdicts(&dir, &verdicts);
    assert_eq!(load_verdicts(&dir), verdicts);

    std::fs::write(dir.join(VERDICTS_FILE), "old-format-line\n1.27.0 audited\n").expect("write");
    assert_eq!(
        load_verdicts(&dir),
        Verdicts::default(),
        "a cache from the hash era is dropped"
    );

    let garbage = format!(
        "{}\n1.26.1 maybe\nnot-a-version untrusted\n1.27.0 trusted short\n1.27.1 trusted {DLL_A} extra\n",
        fingerprint()
    );
    std::fs::write(dir.join(VERDICTS_FILE), garbage).expect("write");
    assert_eq!(load_verdicts(&dir), Verdicts::default());
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn a_new_injector_is_announced_once() {
    let dir = std::env::temp_dir().join(format!("dekan_ltk_notice_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("fixture dir");
    assert!(!already_notified(&dir, "1.27.0"));
    remember_notified(&dir, "1.27.0");
    assert!(already_notified(&dir, "1.27.0"));
    assert!(!already_notified(&dir, "1.28.0"));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[tokio::test]
#[ignore = "downloads LTK Manager releases from GitHub"]
async fn the_newest_signed_release_is_found_online() {
    let dir = std::env::temp_dir().join(format!("dekan_ltk_online_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture may not exist yet
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let version = compatible_version(&dir).await.expect("a signed release");
    let verdicts = load_verdicts(&dir);
    assert!(matches!(
        verdicts.get(&version),
        Some(Injector::Trusted { .. })
    ));
    let _ = std::fs::remove_dir_all(&dir); // ignore-ok: fixture cleanup
}

#[test]
fn pages_point_at_the_ltk_manager_releases() {
    assert_eq!(
        release_page(Some("1.27.0")),
        format!("{LTK_REPOSITORY}/releases/tag/v1.27.0")
    );
    assert_eq!(release_page(None), format!("{LTK_REPOSITORY}/releases"));
}
