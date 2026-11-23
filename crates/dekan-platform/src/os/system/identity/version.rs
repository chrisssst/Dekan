use std::path::{Path, PathBuf};

pub use super::search::discover_installed_exe;

#[must_use]
pub fn display_version() -> &'static str {
    const RAW: &str = env!("CARGO_PKG_VERSION");
    if let Some(stripped) = RAW.strip_suffix(".0") {
        stripped
    } else {
        RAW
    }
}

#[must_use]
pub fn format_version(version: (u16, u16, u16)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

#[must_use]
pub fn is_version_at_least(current: (u16, u16, u16), required: (u16, u16, u16)) -> bool {
    current >= required
}

#[cfg(windows)]
#[must_use]
pub fn read_pe_version(path: &Path) -> Option<(u16, u16, u16)> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };
    use windows::core::PCWSTR;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);

    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None) };
    if size == 0 {
        return None;
    }

    let mut buffer = vec![0u8; size as usize];
    let ok = unsafe {
        GetFileVersionInfoW(
            PCWSTR(wide.as_ptr()),
            Some(0),
            size,
            buffer.as_mut_ptr().cast(),
        )
    };
    if ok.is_err() {
        return None;
    }

    let sub_block: Vec<u16> = "\\\0".encode_utf16().collect();
    let mut info_ptr: *mut c_void = std::ptr::null_mut();
    let mut info_len: u32 = 0;

    let found = unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            PCWSTR(sub_block.as_ptr()),
            &raw mut info_ptr,
            &raw mut info_len,
        )
    };
    let base = buffer.as_ptr() as usize;
    let ptr = info_ptr as usize;
    let needed = std::mem::size_of::<VS_FIXEDFILEINFO>();
    if !found.as_bool() || info_ptr.is_null() || (info_len as usize) < needed || ptr < base {
        return None;
    }
    let offset = ptr - base;
    let slice = buffer.get(offset..offset.checked_add(needed)?)?;
    let sig = u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]);
    if sig != 0xFEEF_04BD {
        return None;
    }
    let ms = u32::from_le_bytes([slice[8], slice[9], slice[10], slice[11]]);
    let ls = u32::from_le_bytes([slice[12], slice[13], slice[14], slice[15]]);
    let major = ((ms >> 16) & 0xFFFF) as u16;
    let minor = (ms & 0xFFFF) as u16;
    let patch = ((ls >> 16) & 0xFFFF) as u16;
    Some((major, minor, patch))
}

#[cfg(not(windows))]
#[must_use]
pub fn read_pe_version(path: &Path) -> Option<(u16, u16, u16)> {
    read_macos_bundle_version(path)
}

#[must_use]
pub fn read_macos_bundle_version(path: &Path) -> Option<(u16, u16, u16)> {
    for ancestor in path.ancestors().take(4) {
        let plist = if ancestor.ends_with("Contents") {
            ancestor.join("Info.plist")
        } else {
            ancestor.join("Contents").join("Info.plist")
        };
        if let Ok(text) = std::fs::read_to_string(&plist)
            && let Some(ver) = parse_plist_short_version(&text)
        {
            return Some(ver);
        }
    }
    None
}

#[must_use]
pub fn parse_plist_short_version(plist_xml: &str) -> Option<(u16, u16, u16)> {
    let key_idx = plist_xml.find("CFBundleShortVersionString")?;
    let after_key = &plist_xml[key_idx..];
    let start_tag = after_key.find("<string>")?;
    let after_start = &after_key[start_tag + "<string>".len()..];
    let end_tag = after_start.find("</string>")?;
    let raw = after_start[..end_tag].trim().trim_start_matches('v');
    let mut parts = raw.split('.');
    let major = parts.next()?.parse::<u16>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u16>().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("0")
        .parse::<u16>()
        .ok()?;
    Some((major, minor, patch))
}

#[must_use]
pub fn parse_registry_path_value(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_quotes = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest)
    } else {
        strip_unquoted_cli_args(trimmed)
    };
    let clean = match without_quotes.rfind(',') {
        Some(idx)
            if !without_quotes[idx + 1..].is_empty()
                && without_quotes[idx + 1..]
                    .trim()
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '-') =>
        {
            &without_quotes[..idx]
        }
        _ => without_quotes,
    };
    let clean = clean.trim().trim_matches('"');
    (!clean.is_empty()).then(|| PathBuf::from(clean))
}

fn strip_unquoted_cli_args(input: &str) -> &str {
    let lower = input.to_ascii_lowercase();
    if let Some(idx) = lower.find(".exe") {
        let end = idx + 4;
        if let Some(after) = input.get(end..)
            && (after.is_empty()
                || after.starts_with(char::is_whitespace)
                || after.starts_with(','))
            && let Some(head) = input.get(..end)
        {
            return head;
        }
    }
    input
}

#[cfg(any(windows, test))]
#[must_use]
pub(crate) fn extract_history_exe_path(value_name: &str, exe_names: &[&str]) -> Option<PathBuf> {
    let trimmed = value_name.trim().trim_matches('"');
    let stripped = trimmed
        .strip_suffix(".FriendlyAppName")
        .or_else(|| trimmed.strip_suffix(".ApplicationCompany"))
        .unwrap_or(trimmed);
    let path = PathBuf::from(stripped);
    matches_exe_name(&path, exe_names).then_some(path)
}

#[cfg(any(windows, test))]
pub(crate) fn matches_exe_name(path: &Path, exe_names: &[&str]) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| exe_names.iter().any(|e| name.eq_ignore_ascii_case(e)))
}

#[cfg(windows)]
pub(crate) fn resolve_exe_from_path(parsed: &Path, exe_names: &[&str]) -> Option<PathBuf> {
    if parsed.is_file() && matches_exe_name(parsed, exe_names) {
        return Some(parsed.to_path_buf());
    }
    let dir = if parsed.is_dir() {
        Some(parsed)
    } else {
        parsed.parent()
    };
    let dir = dir?;
    for exe in exe_names {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_version() {
        let v = display_version();
        assert!(!v.is_empty());
        assert!(v.contains('.'));
    }

    #[test]
    fn test_version_comparison() {
        assert!(is_version_at_least((1, 33, 0), (1, 30, 0)));
        assert!(is_version_at_least((1, 30, 0), (1, 30, 0)));
        assert!(!is_version_at_least((1, 29, 9), (1, 30, 0)));
        assert_eq!(format_version((1, 33, 0)), "1.33.0");
    }

    #[cfg(windows)]
    #[test]
    fn test_read_pe_version_from_notepad() {
        let notepad = Path::new(r"C:\Windows\System32\notepad.exe");
        if notepad.is_file() {
            let ver = read_pe_version(notepad);
            assert!(ver.is_some());
        }
    }

    #[test]
    fn parses_quoted_install_location_and_icon_suffixes() {
        assert_eq!(
            parse_registry_path_value(r#""D:\Apps\LTK Manager""#),
            Some(PathBuf::from(r"D:\Apps\LTK Manager"))
        );
        assert_eq!(
            parse_registry_path_value(r#""D:\Apps\LTK Manager\ltk-manager.exe",0"#),
            Some(PathBuf::from(r"D:\Apps\LTK Manager\ltk-manager.exe"))
        );
        assert_eq!(
            parse_registry_path_value(r"C:\Program Files\LTK Manager\ltk-manager.exe"),
            Some(PathBuf::from(
                r"C:\Program Files\LTK Manager\ltk-manager.exe"
            ))
        );
        assert_eq!(
            parse_registry_path_value(r#""E:\Custom\LTK Manager\ltk-manager.exe" "%1""#),
            Some(PathBuf::from(r"E:\Custom\LTK Manager\ltk-manager.exe"))
        );
        assert_eq!(
            parse_registry_path_value(r"F:\Jogos\LTK Manager\uninstall.exe /S"),
            Some(PathBuf::from(r"F:\Jogos\LTK Manager\uninstall.exe"))
        );
        assert_eq!(parse_registry_path_value("   "), None);
        let plist = "<plist><dict><key>CFBundleShortVersionString</key><string>1.34.2</string></dict></plist>";
        assert_eq!(parse_plist_short_version(plist), Some((1, 34, 2)));
        let exes = ["ltk-manager.exe", "LTK Manager.exe"];
        assert_eq!(
            extract_history_exe_path(r"G:\Custom Path\LTK\ltk-manager.exe.FriendlyAppName", &exes),
            Some(PathBuf::from(r"G:\Custom Path\LTK\ltk-manager.exe"))
        );
        assert_eq!(
            extract_history_exe_path(r"H:\Portable\LTK Manager.exe", &exes),
            Some(PathBuf::from(r"H:\Portable\LTK Manager.exe"))
        );
    }
}
