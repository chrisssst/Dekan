#[cfg(any(windows, test))]
use std::path::Path;
use std::path::PathBuf;

#[cfg(any(windows, test))]
use super::version::matches_exe_name;

#[cfg(windows)]
#[must_use]
pub fn discover_installed_exe(keywords: &[&str], exe_names: &[&str]) -> Option<PathBuf> {
    if let Some(found) = super::registry::discover_from_registry(keywords, exe_names) {
        return Some(found);
    }
    if let Some(found) = discover_from_shortcuts(keywords, exe_names) {
        return Some(found);
    }
    if let Some(found) = discover_from_path_env(exe_names) {
        return Some(found);
    }
    discover_from_fixed_drives(keywords, exe_names)
}

#[cfg(not(windows))]
#[must_use]
pub fn discover_installed_exe(_keywords: &[&str], _exe_names: &[&str]) -> Option<PathBuf> {
    None
}

#[cfg(any(windows, test))]
#[must_use]
pub(crate) fn extract_lnk_exe_targets(bytes: &[u8], exe_names: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect_ascii_drive_paths(bytes, exe_names, &mut found);
    collect_utf16le_drive_paths(bytes, exe_names, &mut found);
    found
}

#[cfg(any(windows, test))]
fn collect_ascii_drive_paths(bytes: &[u8], exe_names: &[&str], out: &mut Vec<PathBuf>) {
    let mut i = 0usize;
    while i + 6 < bytes.len() {
        if bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':' && bytes[i + 2] == b'\\' {
            let start = i;
            let mut end = i + 3;
            while end < bytes.len() && end - start < 260 {
                let b = bytes[end];
                if b < 0x20 || b == 0x7F || matches!(b, b'"' | b'<' | b'>' | b'|' | b'*' | b'?') {
                    break;
                }
                end += 1;
            }
            if let Ok(text) = std::str::from_utf8(&bytes[start..end]) {
                push_matching_exe_candidate(text, exe_names, out);
            }
            i = end;
        } else {
            i += 1;
        }
    }
}

#[cfg(any(windows, test))]
fn collect_utf16le_drive_paths(bytes: &[u8], exe_names: &[&str], out: &mut Vec<PathBuf>) {
    let mut i = 0usize;
    while i + 12 < bytes.len() {
        if bytes[i].is_ascii_alphabetic()
            && bytes[i + 1] == 0
            && bytes[i + 2] == b':'
            && bytes[i + 3] == 0
            && bytes[i + 4] == b'\\'
            && bytes[i + 5] == 0
        {
            let mut units = Vec::new();
            let mut j = i;
            while j + 1 < bytes.len() && units.len() < 260 {
                let u = u16::from_le_bytes([bytes[j], bytes[j + 1]]);
                if u < 0x20 || matches!(u, 0x0022 | 0x003C | 0x003E | 0x007C | 0x002A | 0x003F) {
                    break;
                }
                units.push(u);
                j += 2;
            }
            let text = String::from_utf16_lossy(&units);
            push_matching_exe_candidate(&text, exe_names, out);
            i = j.max(i + 2);
        } else {
            i += 1;
        }
    }
}

#[cfg(any(windows, test))]
fn push_matching_exe_candidate(text: &str, exe_names: &[&str], out: &mut Vec<PathBuf>) {
    let lower = text.to_ascii_lowercase();
    for exe in exe_names {
        let exe_lower = exe.to_ascii_lowercase();
        if let Some(pos) = lower.find(&exe_lower) {
            let end = pos + exe_lower.len();
            if let Some(slice) = text.get(..end) {
                let cand = PathBuf::from(slice.trim());
                if matches_exe_name(&cand, exe_names) && !out.contains(&cand) {
                    out.push(cand);
                }
            }
        }
    }
}

#[cfg(windows)]
fn desktop_user_profile_dirs() -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    let Ok(local) = crate::user_profile::local_app_data() else {
        return (None, None, None);
    };
    let appdata = local.parent();
    let roaming = appdata.map(|a| a.join("Roaming"));
    let home = appdata.and_then(|a| a.parent()).map(Path::to_path_buf);
    (Some(local), roaming, home)
}

#[cfg(windows)]
fn discover_from_shortcuts(keywords: &[&str], exe_names: &[&str]) -> Option<PathBuf> {
    let mut roots = Vec::new();
    let (_, desktop_roaming, desktop_home) = desktop_user_profile_dirs();
    for base in [
        std::env::var("PROGRAMDATA").ok().map(PathBuf::from),
        desktop_roaming,
        std::env::var("APPDATA").ok().map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    {
        roots.push(
            base.join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    for base in [
        desktop_home,
        std::env::var("USERPROFILE").ok().map(PathBuf::from),
        std::env::var("PUBLIC").ok().map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    {
        roots.push(base.join("Desktop"));
    }

    for root in roots {
        if let Some(found) = scan_shortcuts_dir(&root, 2, keywords, exe_names) {
            return Some(found);
        }
    }
    None
}

#[cfg(windows)]
fn scan_shortcuts_dir(
    dir: &Path,
    depth: u8,
    keywords: &[&str],
    exe_names: &[&str],
) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() && depth > 0 {
            if let Some(found) = scan_shortcuts_dir(&path, depth - 1, keywords, exe_names) {
                return Some(found);
            }
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".lnk") {
            continue;
        }
        if !keywords.iter().any(|kw| lower.contains(kw)) && !lower.contains("ltk") {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            for candidate in extract_lnk_exe_targets(&bytes, exe_names) {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn discover_from_path_env(exe_names: &[&str]) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        if let Some(found) = check_dir_for_exes(&dir, exe_names) {
            return Some(found);
        }
    }
    None
}

#[cfg(windows)]
fn fixed_drive_roots() -> Vec<PathBuf> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows::core::PCWSTR;

    const DRIVE_FIXED: u32 = 3;
    let mask = unsafe { GetLogicalDrives() };
    let mut roots = Vec::new();
    for idx in 0..26u32 {
        if (mask & (1 << idx)) == 0 {
            continue;
        }
        let letter = (b'A' + idx as u8) as char;
        let root_str = format!("{letter}:\\");
        let wide: Vec<u16> = root_str.encode_utf16().chain(std::iter::once(0)).collect();
        if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } == DRIVE_FIXED {
            roots.push(PathBuf::from(root_str));
        }
    }
    roots
}

#[cfg(windows)]
fn discover_from_fixed_drives(keywords: &[&str], exe_names: &[&str]) -> Option<PathBuf> {
    let mut scan_roots = Vec::new();
    for env_var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(base) = std::env::var(env_var) {
            scan_roots.push(PathBuf::from(base));
        }
    }
    let (desktop_local, desktop_roaming, desktop_home) = desktop_user_profile_dirs();
    if let Some(local) = desktop_local {
        scan_roots.push(local.join("Programs"));
        scan_roots.push(local);
    }
    if let Some(roaming) = desktop_roaming {
        scan_roots.push(roaming);
    }
    if let Some(home) = desktop_home {
        scan_roots.push(home.join("Desktop"));
        scan_roots.push(home.join("Downloads"));
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        let base = PathBuf::from(la);
        scan_roots.push(base.join("Programs"));
        scan_roots.push(base);
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        let base = PathBuf::from(home);
        scan_roots.push(base.join("Desktop"));
        scan_roots.push(base.join("Downloads"));
    }

    let common_subdirs = [
        "LTK Manager",
        "LeagueToolkit",
        "Program Files",
        "Program Files (x86)",
        "Programs",
        "Apps",
        "Games",
        "Jogos",
        "Riot Games",
    ];
    for drive in fixed_drive_roots() {
        if let Some(found) = check_dir_for_exes(&drive, exe_names) {
            return Some(found);
        }
        for sub in common_subdirs {
            let dir = drive.join(sub);
            if let Some(found) = check_dir_for_exes(&dir, exe_names) {
                return Some(found);
            }
            if dir.is_dir() {
                scan_roots.push(dir);
            }
        }
        scan_roots.push(drive);
    }

    let mut seen = std::collections::HashSet::new();
    for root in scan_roots {
        let norm = root.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(norm) {
            continue;
        }
        if let Some(found) = scan_matching_folders(&root, 2, keywords, exe_names) {
            return Some(found);
        }
    }
    None
}

#[cfg(any(windows, test))]
fn check_dir_for_exes(dir: &Path, exe_names: &[&str]) -> Option<PathBuf> {
    for exe in exe_names {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(any(windows, test))]
pub(crate) fn scan_matching_folders(
    root: &Path,
    depth: u8,
    keywords: &[&str],
    exe_names: &[&str],
) -> Option<PathBuf> {
    if let Some(found) = check_dir_for_exes(root, exe_names) {
        return Some(found);
    }
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let path = entry.path();
        if let Some(found) = check_dir_for_exes(&path, exe_names) {
            return Some(found);
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let interesting = keywords.iter().any(|kw| lower.contains(kw))
            || lower.contains("ltk")
            || lower.contains("toolkit")
            || lower.contains("mod");
        if interesting
            && let Some(found) = scan_matching_folders(&path, depth - 1, keywords, exe_names)
        {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ascii_and_utf16le_paths_from_lnk_payload() {
        let exes = ["ltk-manager.exe", "LTK Manager.exe"];
        let mut blob = vec![0u8; 16];
        blob.extend_from_slice(br"D:\Games\LTK Manager\ltk-manager.exe");
        blob.push(0);
        let wide = r"E:\Custom Folder\LTK\LTK Manager.exe";
        for u in wide.encode_utf16() {
            blob.extend_from_slice(&u.to_le_bytes());
        }
        blob.extend_from_slice(&[0, 0]);

        let targets = extract_lnk_exe_targets(&blob, &exes);
        assert!(targets.contains(&PathBuf::from(r"D:\Games\LTK Manager\ltk-manager.exe")));
        assert!(targets.contains(&PathBuf::from(r"E:\Custom Folder\LTK\LTK Manager.exe")));
    }

    #[test]
    fn shallow_folder_scan_finds_custom_install_directory() {
        let root = std::env::temp_dir().join(format!("dekan_scan_ltk_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup before test
        let nested = root.join("My LTK Custom").join("bin");
        std::fs::create_dir_all(&nested).expect("nested dir");
        let exe = nested.join("ltk-manager.exe");
        std::fs::write(&exe, b"mz").expect("write exe");

        let found = scan_matching_folders(
            &root,
            2,
            &["ltk manager", "leaguetoolkit"],
            &["ltk-manager.exe"],
        );
        assert_eq!(found, Some(exe));
        let _ = std::fs::remove_dir_all(&root); // ignore-ok: cleanup after test
    }
}
