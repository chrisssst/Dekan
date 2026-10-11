use super::*;

const MANIFEST: &str = "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"1.2.1\"\nedition = \"2024\"\n\n[workspace.dependencies]\ntokio = { version = \"1\", features = [\"full\"] }\n";

const LOCK: &str = "version = 4\n\n[[package]]\nname = \"dekan-app\"\nversion = \"1.2.1\"\ndependencies = [\n \"tokio\",\n]\n\n[[package]]\nname = \"tokio\"\nversion = \"1.47.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"abc\"\n\n[[package]]\nname = \"xtask\"\nversion = \"1.2.1\"\n";

#[test]
fn only_two_number_versions_are_accepted_and_they_gain_a_zero_patch() {
    assert_eq!(cargo_version("1.3").as_deref(), Some("1.3.0"));
    assert_eq!(cargo_version("2.0").as_deref(), Some("2.0.0"));
    for bad in ["1", "1.3.1", "v1.3", "1.x", "", "1..3"] {
        assert_eq!(cargo_version(bad), None, "{bad:?}");
    }
}

#[test]
fn the_manifest_changes_only_the_workspace_version() {
    let stamped = stamp_manifest(MANIFEST, "1.3.0").expect("stamped");
    assert!(stamped.contains("[workspace.package]\nversion = \"1.3.0\"\n"));
    assert!(
        stamped.contains("tokio = { version = \"1\""),
        "dependency versions are untouched"
    );
    assert_eq!(stamped.lines().count(), MANIFEST.lines().count());
    assert!(stamp_manifest("[package]\nversion = \"1.0.0\"\n", "1.3.0").is_err());

    let crlf = MANIFEST.replace('\n', "\r\n");
    let stamped = stamp_manifest(&crlf, "1.3.0").expect("stamped");
    assert!(
        stamped.contains("version = \"1.3.0\"\r\n"),
        "line endings are kept"
    );
}

#[test]
fn the_lock_file_changes_every_member_and_no_registry_crate() {
    let stamped = stamp_lock(LOCK, "1.3.0").expect("stamped");
    assert!(stamped.contains("name = \"dekan-app\"\nversion = \"1.3.0\""));
    assert!(stamped.contains("name = \"xtask\"\nversion = \"1.3.0\""));
    assert!(
        stamped.contains("name = \"tokio\"\nversion = \"1.47.0\""),
        "registry crates keep their version"
    );
    assert!(
        stamped.starts_with("version = 4\n"),
        "the lock format line is untouched"
    );
    let changed: Vec<(&str, &str)> = LOCK
        .lines()
        .zip(stamped.lines())
        .filter(|(before, after)| before != after)
        .collect();
    assert_eq!(
        changed,
        vec![("version = \"1.2.1\"", "version = \"1.3.0\""); 2],
        "only the members' version lines change"
    );
    assert_eq!(stamped.len(), LOCK.len());
    assert!(stamp_lock("version = 4\n", "1.3.0").is_err());

    let crlf = LOCK.replace('\n', "\r\n");
    let stamped_crlf = stamp_lock(&crlf, "1.3.0").expect("a CRLF lock file is stamped too");
    assert_eq!(stamped_crlf, stamped.replace('\n', "\r\n"));
}
