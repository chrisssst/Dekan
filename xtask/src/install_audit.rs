use std::path::{Path, PathBuf};

pub const UNINSTALL_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{D387A5B1-8C56-4D2A-94B8-975DE11C6B45}_is1";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const LAYERS_KEYS: [&str; 2] = [
    r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers",
    r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers",
];

const USER_CONTENT: [&str; 3] = ["library", "skins", "custom_mods"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Installed,
    Uninstalled { kept_user_content: bool },
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub program_dir: PathBuf,

    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub ok: bool,
    pub what: String,
}

fn check(ok: bool, what: impl Into<String>) -> Check {
    Check {
        ok,
        what: what.into(),
    }
}

pub trait Registry {
    fn key_exists(&self, key: &str) -> bool;
    fn value_exists(&self, key: &str, value: &str) -> bool;
    fn read_value(&self, key: &str, value: &str) -> Option<String>;
}

#[must_use]
pub fn install_location(registry: &dyn Registry) -> Option<PathBuf> {
    registry
        .read_value(UNINSTALL_KEY, "InstallLocation")
        .map(|s| s.trim().trim_matches('"').to_owned())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

pub fn audit_files(
    layout: &Layout,
    phase: Phase,
    tools: &[(&str, &str)],
    sha256: &dyn Fn(&Path) -> Option<String>,
) -> Vec<Check> {
    let mut checks = Vec::new();
    match phase {
        Phase::Installed => {
            let exe = layout.program_dir.join("dekan.exe");
            checks.push(check(exe.is_file(), format!("{} present", exe.display())));
            let tools_dir = layout.program_dir.join("tools");
            for (name, audited) in tools {
                let path = tools_dir.join(name);
                match sha256(&path) {
                    Some(actual) => checks.push(check(
                        actual.eq_ignore_ascii_case(audited),
                        format!("{} has the audited hash (found {actual})", path.display()),
                    )),
                    None => checks.push(check(false, format!("{} present", path.display()))),
                }
            }
            let strays: Vec<String> = std::fs::read_dir(&tools_dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|name| !tools.iter().any(|(t, _)| t.eq_ignore_ascii_case(name)))
                        .collect()
                })
                .unwrap_or_default();
            checks.push(check(
                strays.is_empty(),
                format!(
                    "no file in {} beyond the shipped tools {strays:?}",
                    tools_dir.display()
                ),
            ));
        }
        Phase::Uninstalled { kept_user_content } => {
            checks.push(check(
                !layout.program_dir.exists(),
                format!("{} removed", layout.program_dir.display()),
            ));
            let left: Vec<String> = std::fs::read_dir(&layout.data_dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|name| {
                            !(kept_user_content
                                && USER_CONTENT.iter().any(|u| u.eq_ignore_ascii_case(name)))
                        })
                        .collect()
                })
                .unwrap_or_default();
            checks.push(check(
                left.is_empty(),
                format!(
                    "nothing left in {} {}(found {left:?})",
                    layout.data_dir.display(),
                    if kept_user_content {
                        "but the kept skins and mods "
                    } else {
                        ""
                    }
                ),
            ));
        }
    }
    checks
}

pub fn audit_registry(phase: Phase, exe: &Path, registry: &dyn Registry) -> Vec<Check> {
    let exe = exe.to_string_lossy();
    let mut checks = Vec::new();
    let installed = phase == Phase::Installed;
    checks.push(check(
        registry.key_exists(UNINSTALL_KEY) == installed,
        format!(
            "uninstall entry {}",
            if installed { "present" } else { "removed" }
        ),
    ));
    for key in LAYERS_KEYS {
        checks.push(check(
            !registry.value_exists(key, &exe),
            format!("no compatibility flag (Run as administrator) at {key}"),
        ));
    }
    if !installed {
        checks.push(check(
            !registry.value_exists(RUN_KEY, "Dekan"),
            "no \"Start with Windows\" entry left",
        ));
    }
    checks
}

pub struct RegExe;

impl RegExe {
    fn query(args: &[&str]) -> bool {
        let reg = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32")
            .join("reg.exe");
        std::process::Command::new(reg)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

impl RegExe {
    fn query_value(key: &str, value: &str) -> Option<String> {
        let reg = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32")
            .join("reg.exe");
        let out = std::process::Command::new(reg)
            .args(["query", key, "/v", value, "/reg:64"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some(rest) = line.trim_start().strip_prefix(value) {
                if let Some((_type, data)) = rest.trim_start().split_once("    ") {
                    let data = data.trim();
                    if !data.is_empty() {
                        return Some(data.to_owned());
                    }
                }
            }
        }
        None
    }
}

impl Registry for RegExe {
    fn key_exists(&self, key: &str) -> bool {
        Self::query(&["query", key, "/reg:64"])
    }

    fn value_exists(&self, key: &str, value: &str) -> bool {
        Self::query(&["query", key, "/v", value, "/reg:64"])
    }

    fn read_value(&self, key: &str, value: &str) -> Option<String> {
        Self::query_value(key, value)
    }
}

pub fn report(checks: &[Check]) -> bool {
    for c in checks {
        println!("[{}] {}", if c.ok { " OK " } else { "FAIL" }, c.what);
    }
    let failed = checks.iter().filter(|c| !c.ok).count();
    println!(
        "\n{} checks, {failed} failed",
        checks.len(),
        failed = failed
    );
    failed == 0
}

pub fn parse_phase(args: &[String]) -> Option<Phase> {
    let kept = args.iter().any(|a| a == "--kept-user-content");
    match args.first().map(String::as_str) {
        Some("installed") => Some(Phase::Installed),
        Some("uninstalled") => Some(Phase::Uninstalled {
            kept_user_content: kept,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct Temp(PathBuf);
    impl Temp {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "dekan_install_audit_{name}_{}",
                std::process::id()
            ));
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

    fn content_hash(path: &Path) -> Option<String> {
        std::fs::read(path)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    fn layout(root: &Path) -> Layout {
        Layout {
            program_dir: root.join("Program Files").join("Dekan"),
            data_dir: root.join("LocalAppData").join("Dekan"),
        }
    }

    const TOOLS: [(&str, &str); 1] = [("ltk_patcher_host.exe", "h1")];

    #[test]
    fn test_a_correct_install_passes_and_a_wrong_tool_or_stray_fails() {
        let root = Temp::new("installed");
        let layout = layout(&root.0);
        let tools = layout.program_dir.join("tools");
        std::fs::create_dir_all(&tools).expect("dir");
        std::fs::write(layout.program_dir.join("dekan.exe"), b"exe").expect("exe");
        std::fs::write(tools.join("ltk_patcher_host.exe"), b"h1").expect("host");
        let checks = audit_files(&layout, Phase::Installed, &TOOLS, &content_hash);
        assert!(checks.iter().all(|c| c.ok), "{checks:#?}");

        std::fs::write(tools.join("cloudflared.exe"), b"x").expect("stray");
        let failed: Vec<String> = audit_files(&layout, Phase::Installed, &TOOLS, &content_hash)
            .into_iter()
            .filter(|c| !c.ok)
            .map(|c| c.what)
            .collect();
        assert_eq!(failed.len(), 1, "{failed:#?}");
        assert!(failed[0].contains("cloudflared.exe"));
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
            &content_hash,
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
        let checks = audit_files(&layout, kept, &TOOLS, &content_hash);
        let failed: Vec<&Check> = checks.iter().filter(|c| !c.ok).collect();
        assert_eq!(failed.len(), 1, "{checks:#?}");
        assert!(failed[0].what.contains("overlay"), "the overlay is residue");
        assert!(!failed[0].what.contains("library"), "kept skins are not");

        std::fs::remove_dir_all(layout.data_dir.join("overlay")).expect("rm");
        assert!(
            audit_files(&layout, kept, &TOOLS, &content_hash)
                .iter()
                .all(|c| c.ok)
        );
        let not_kept = Phase::Uninstalled {
            kept_user_content: false,
        };
        assert!(
            audit_files(&layout, not_kept, &TOOLS, &content_hash)
                .iter()
                .any(|c| !c.ok),
            "skins left when the user asked to remove them are residue"
        );

        std::fs::create_dir_all(&layout.program_dir).expect("pf");
        assert!(
            audit_files(&layout, kept, &TOOLS, &content_hash)
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
}
