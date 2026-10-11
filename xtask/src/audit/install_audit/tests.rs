use super::*;
use std::collections::HashSet;

struct Temp(PathBuf);
impl Temp {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("dekan_install_audit_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path); // ignore-ok: fixture may not exist yet
        std::fs::create_dir_all(&path).expect("temp");
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0); // ignore-ok: fixture cleanup
    }
}

#[derive(Default)]
struct FakeRegistry {
    keys: HashSet<String>,
    values: HashSet<(String, String)>,
    strings: std::collections::HashMap<(String, String), String>,
}
impl Registry for FakeRegistry {
    fn key_exists(&self, key: &str) -> bool {
        self.keys.contains(key)
    }
    fn value_exists(&self, key: &str, value: &str) -> bool {
        self.values.contains(&(key.to_owned(), value.to_owned()))
    }
    fn read_value(&self, key: &str, value: &str) -> Option<String> {
        self.strings
            .get(&(key.to_owned(), value.to_owned()))
            .cloned()
    }
}

fn signed_marker(path: &Path) -> Result<(), String> {
    match std::fs::read(path) {
        Ok(bytes) if bytes == b"signed" => Ok(()),
        Ok(_) => Err("not signed".to_owned()),
        Err(e) => Err(e.to_string()),
    }
}

fn layout(root: &Path) -> Layout {
    Layout {
        program_dir: root.join("Program Files").join("Dekan"),
        data_dir: root.join("LocalAppData").join("Dekan"),
    }
}

const TOOLS: [&str; 1] = ["ltk_patcher_host.exe"];

#[test]
fn test_a_correct_install_passes_and_a_wrong_tool_or_stray_fails() {
    let root = Temp::new("installed");
    let layout = layout(&root.0);
    let tools = layout.program_dir.join("tools");
    std::fs::create_dir_all(&tools).expect("dir");
    std::fs::write(layout.program_dir.join("dekan.exe"), b"exe").expect("exe");
    std::fs::write(tools.join("ltk_patcher_host.exe"), b"signed").expect("host");
    let checks = audit_files(&layout, Phase::Installed, &TOOLS, &signed_marker);
    assert!(checks.iter().all(|c| c.ok), "{checks:#?}");

    std::fs::write(tools.join("cloudflared.exe"), b"x").expect("stray");
    let failed: Vec<String> = audit_files(&layout, Phase::Installed, &TOOLS, &signed_marker)
        .into_iter()
        .filter(|c| !c.ok)
        .map(|c| c.what)
        .collect();
    assert_eq!(failed.len(), 1, "{failed:#?}");
    assert!(failed[0].contains("cloudflared.exe"));

    std::fs::remove_file(tools.join("cloudflared.exe")).expect("rm stray");
    std::fs::write(tools.join("ltk_patcher_host.exe"), b"patched").expect("patched");
    let failed: Vec<String> = audit_files(&layout, Phase::Installed, &TOOLS, &signed_marker)
        .into_iter()
        .filter(|c| !c.ok)
        .map(|c| c.what)
        .collect();
    assert_eq!(failed.len(), 1, "{failed:#?}");
    assert!(failed[0].contains("not signed"), "{failed:#?}");
}

#[test]
fn test_uninstall_residue_is_reported_and_kept_user_content_is_allowed() {
    let root = Temp::new("uninstalled");
    let layout = layout(&root.0);
    let clean = audit_files(
        &layout,
        Phase::Uninstalled {
            kept_user_content: false,
        },
        &TOOLS,
        &signed_marker,
    );
    assert!(
        clean.iter().all(|c| c.ok),
        "nothing there is clean: {clean:#?}"
    );

    std::fs::create_dir_all(layout.data_dir.join("library")).expect("lib");
    std::fs::create_dir_all(layout.data_dir.join("overlay")).expect("overlay");
    let kept = Phase::Uninstalled {
        kept_user_content: true,
    };
    let checks = audit_files(&layout, kept, &TOOLS, &signed_marker);
    let failed: Vec<&Check> = checks.iter().filter(|c| !c.ok).collect();
    assert_eq!(failed.len(), 1, "{checks:#?}");
    assert!(failed[0].what.contains("overlay"), "the overlay is residue");
    assert!(!failed[0].what.contains("library"), "kept skins are not");

    std::fs::remove_dir_all(layout.data_dir.join("overlay")).expect("rm");
    assert!(
        audit_files(&layout, kept, &TOOLS, &signed_marker)
            .iter()
            .all(|c| c.ok)
    );
    let not_kept = Phase::Uninstalled {
        kept_user_content: false,
    };
    assert!(
        audit_files(&layout, not_kept, &TOOLS, &signed_marker)
            .iter()
            .any(|c| !c.ok),
        "skins left when the user asked to remove them are residue"
    );

    std::fs::create_dir_all(&layout.program_dir).expect("pf");
    assert!(
        audit_files(&layout, kept, &TOOLS, &signed_marker)
            .iter()
            .any(|c| !c.ok && c.what.contains("Program Files"))
    );
}

#[test]
fn test_registry_rules_for_both_phases() {
    let exe = Path::new(r"C:\Program Files\Dekan\dekan.exe");
    let mut registry = FakeRegistry::default();
    registry.keys.insert(UNINSTALL_KEY.to_owned());
    assert!(
        audit_registry(Phase::Installed, exe, &registry)
            .iter()
            .all(|c| c.ok)
    );

    registry.values.insert((
        LAYERS_KEYS[1].to_owned(),
        exe.to_string_lossy().into_owned(),
    ));
    let failed: Vec<Check> = audit_registry(Phase::Installed, exe, &registry)
        .into_iter()
        .filter(|c| !c.ok)
        .collect();
    assert_eq!(failed.len(), 1);
    assert!(
        failed[0]
            .what
            .contains(r"HKCU\Software\Microsoft\Windows NT")
    );

    let uninstalled = Phase::Uninstalled {
        kept_user_content: false,
    };
    let mut after = FakeRegistry::default();
    assert!(
        audit_registry(uninstalled, exe, &after)
            .iter()
            .all(|c| c.ok)
    );
    after.keys.insert(UNINSTALL_KEY.to_owned());
    after
        .values
        .insert((RUN_KEY.to_owned(), "Dekan".to_owned()));
    let failed = audit_registry(uninstalled, exe, &after)
        .into_iter()
        .filter(|c| !c.ok)
        .count();
    assert_eq!(failed, 2, "uninstall entry and autostart value are residue");
}

#[test]
fn test_install_location_comes_from_the_registry_never_a_fixed_drive() {
    let mut registry = FakeRegistry::default();
    assert_eq!(
        install_location(&registry),
        None,
        "absent when not recorded"
    );
    registry.strings.insert(
        (UNINSTALL_KEY.to_owned(), "InstallLocation".to_owned()),
        r"D:\Apps\Dekan".to_owned(),
    );
    assert_eq!(
        install_location(&registry),
        Some(PathBuf::from(r"D:\Apps\Dekan")),
        "a non-default install folder is honored"
    );

    registry.strings.insert(
        (UNINSTALL_KEY.to_owned(), "InstallLocation".to_owned()),
        "  \"E:\\Games\\Dekan\"  ".to_owned(),
    );
    assert_eq!(
        install_location(&registry),
        Some(PathBuf::from(r"E:\Games\Dekan"))
    );
}

#[test]
fn test_phase_parsing() {
    let args = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(parse_phase(&args(&["installed"])), Some(Phase::Installed));
    assert_eq!(
        parse_phase(&args(&["uninstalled", "--kept-user-content"])),
        Some(Phase::Uninstalled {
            kept_user_content: true
        })
    );
    assert_eq!(parse_phase(&args(&[])), None);
    assert_eq!(parse_phase(&args(&["whatever"])), None);
}
