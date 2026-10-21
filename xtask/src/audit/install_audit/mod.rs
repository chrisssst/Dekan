use crate::dekan_data_dir;
use crate::package::USER_SUPPLIED_TOOLS;

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
    tools: &[&str],
    trusted: &dyn Fn(&Path) -> Result<(), String>,
) -> Vec<Check> {
    let mut checks = Vec::new();
    match phase {
        Phase::Installed => {
            let exe = layout.program_dir.join("dekan.exe");
            checks.push(check(exe.is_file(), format!("{} present", exe.display())));
            let tools_dir = layout.program_dir.join("tools");
            for name in tools {
                let path = tools_dir.join(name);
                if !path.is_file() {
                    checks.push(check(false, format!("{} present", path.display())));
                    continue;
                }
                let verdict = trusted(&path);
                checks.push(check(
                    verdict.is_ok(),
                    format!(
                        "{} is signed by {} ({})",
                        path.display(),
                        dekan_inject::trust::LTK_PUBLISHER,
                        verdict.err().unwrap_or_else(|| "verified".to_owned())
                    ),
                ));
            }
            let (strays, listed) = match list_names(&tools_dir) {
                Ok(names) => (
                    names
                        .into_iter()
                        .filter(|name| !tools.iter().any(|t| t.eq_ignore_ascii_case(name)))
                        .collect::<Vec<String>>(),
                    true,
                ),
                Err(e) => (vec![e], false),
            };
            checks.push(check(
                listed && strays.is_empty(),
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
            let (left, listed) = match list_names(&layout.data_dir) {
                Ok(names) => (
                    names
                        .into_iter()
                        .filter(|name| {
                            !(kept_user_content
                                && USER_CONTENT.iter().any(|u| u.eq_ignore_ascii_case(name)))
                        })
                        .collect::<Vec<String>>(),
                    true,
                ),
                Err(e) => (vec![e], false),
            };
            checks.push(check(
                listed && left.is_empty(),
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

fn list_names(dir: &std::path::Path) -> Result<Vec<String>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{} unreadable: {e}", dir.display())),
    };
    let mut names = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => names.push(entry.file_name().to_string_lossy().into_owned()),
            Err(e) => return Err(format!("{} unreadable: {e}", dir.display())),
        }
    }
    Ok(names)
}

pub(crate) fn run_install_audit(args: &[String]) {
    let Some(phase) = parse_phase(args) else {
        eprintln!("usage: cargo xtask install-audit installed | uninstalled [--kept-user-content]");
        std::process::exit(2);
    };

    let registry = RegExe;
    let program_dir = install_location(&registry).unwrap_or_else(|| {
        std::env::var_os("ProgramW6432")
            .or_else(|| std::env::var_os("ProgramFiles"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
            .join("Dekan")
    });
    let layout = Layout {
        program_dir,
        data_dir: dekan_data_dir(),
    };
    let tools_dir = layout.program_dir.join("tools");
    let mut tools = Vec::with_capacity(USER_SUPPLIED_TOOLS.len());
    for name in USER_SUPPLIED_TOOLS {
        if tools_dir.join(name).is_file() {
            tools.push(name);
        } else {
            println!(
                "  [INFO] {name} not in {} yet (the user supplies it)",
                tools_dir.display()
            );
        }
    }
    let trusted = |path: &std::path::Path| -> Result<(), String> {
        dekan_inject::trust::verify_injector_file(path).map_err(|e| e.to_string())
    };
    let mut checks = audit_files(&layout, phase, &tools, &trusted);
    checks.extend(audit_registry(
        phase,
        &layout.program_dir.join("dekan.exe"),
        &registry,
    ));
    if !report(&checks) {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
