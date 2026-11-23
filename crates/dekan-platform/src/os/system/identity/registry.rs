#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use super::version::{extract_history_exe_path, parse_registry_path_value, resolve_exe_from_path};

#[cfg(windows)]
#[must_use]
pub(crate) fn discover_from_registry(keywords: &[&str], exe_names: &[&str]) -> Option<PathBuf> {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };

    let views = [KEY_READ | KEY_WOW64_64KEY, KEY_READ | KEY_WOW64_32KEY];
    let hives = [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE];

    for hive in hives {
        for view in views {
            if let Some(found) = scan_uninstall_root(
                hive,
                r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
                view,
                keywords,
                exe_names,
            ) {
                return Some(found);
            }
        }
    }

    for hive in hives {
        if let Some(found) = scan_shell_and_app_paths(hive, exe_names) {
            return Some(found);
        }
        for view in views {
            for subkey in [r"Software\LTK Manager", r"Software\LeagueToolkit\Manager"] {
                if let Some(found) = read_vendor_key(hive, subkey, view, exe_names) {
                    return Some(found);
                }
            }
        }
    }

    for history_key in [
        r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache",
        r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Compatibility Assistant\Store",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\FeatureUsage\AppSwitched",
    ] {
        if let Some(found) = scan_value_names_for_exe(HKEY_CURRENT_USER, history_key, exe_names) {
            return Some(found);
        }
    }

    None
}

#[cfg(windows)]
unsafe fn read_reg_str(hkey: windows::Win32::System::Registry::HKEY, name: &str) -> Option<String> {
    use windows::Win32::System::Registry::{RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::PCWSTR;

    let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = vec![0u16; 512];
    let mut bytes = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            hkey,
            PCWSTR::null(),
            PCWSTR(wide_name.as_ptr()),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status.is_err() {
        return None;
    }
    let chars = (bytes as usize / 2).min(buf.len());
    let slice = &buf[..chars];
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    Some(String::from_utf16_lossy(&slice[..end]))
}

#[cfg(windows)]
fn scan_uninstall_root(
    hive: windows::Win32::System::Registry::HKEY,
    subkey_path: &str,
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
    keywords: &[&str],
    exe_names: &[&str],
) -> Option<PathBuf> {
    use windows::Win32::Foundation::ERROR_NO_MORE_ITEMS;
    use windows::Win32::System::Registry::{HKEY, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW};
    use windows::core::PCWSTR;

    let subkey_wide: Vec<u16> = subkey_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut root_key = HKEY::default();
    if unsafe {
        RegOpenKeyExW(
            hive,
            PCWSTR(subkey_wide.as_ptr()),
            Some(0),
            access,
            &mut root_key,
        )
    }
    .is_err()
    {
        return None;
    }

    let mut found = None;
    let mut index = 0u32;
    let mut name_buf = [0u16; 256];
    loop {
        let mut name_len = name_buf.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                root_key,
                index,
                Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS || status.is_err() {
            break;
        }
        index += 1;

        let child_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
        let child_wide: Vec<u16> = child_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut app_key = HKEY::default();
        if unsafe {
            RegOpenKeyExW(
                root_key,
                PCWSTR(child_wide.as_ptr()),
                Some(0),
                access,
                &mut app_key,
            )
        }
        .is_err()
        {
            continue;
        }

        let display_name = unsafe { read_reg_str(app_key, "DisplayName") }
            .unwrap_or_default()
            .to_ascii_lowercase();
        let publisher = unsafe { read_reg_str(app_key, "Publisher") }
            .unwrap_or_default()
            .to_ascii_lowercase();
        let key_lower = child_name.to_ascii_lowercase();
        let matched = keywords.iter().any(|kw| {
            display_name.contains(kw) || publisher.contains(kw) || key_lower.contains(kw)
        });
        if matched {
            for field in [
                "DisplayIcon",
                "InstallLocation",
                "UninstallString",
                "QuietUninstallString",
            ] {
                if let Some(raw) = unsafe { read_reg_str(app_key, field) }
                    && let Some(parsed) = parse_registry_path_value(&raw)
                    && let Some(candidate) = resolve_exe_from_path(&parsed, exe_names)
                {
                    found = Some(candidate);
                    break;
                }
            }
        }
        let _ = unsafe { RegCloseKey(app_key) }; // ignore-ok: close app uninstall key
        if found.is_some() {
            break;
        }
    }
    let _ = unsafe { RegCloseKey(root_key) }; // ignore-ok: close uninstall root key
    found
}

#[cfg(windows)]
fn scan_shell_and_app_paths(
    hive: windows::Win32::System::Registry::HKEY,
    exe_names: &[&str],
) -> Option<PathBuf> {
    use windows::Win32::System::Registry::{KEY_READ, KEY_WOW64_64KEY};

    let mut keys = vec![
        (
            r"Software\Classes\LTKManager.Fantome\shell\open\command".to_owned(),
            "",
        ),
        (
            r"Software\Classes\LTKManager.Modpkg\shell\open\command".to_owned(),
            "",
        ),
    ];
    for exe in exe_names {
        keys.push((
            format!(r"Software\Classes\Applications\{exe}\shell\open\command"),
            "",
        ));
        keys.push((
            format!(r"Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}"),
            "",
        ));
        keys.push((
            format!(r"Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}"),
            "Path",
        ));
    }

    for (subkey, field) in keys {
        if let Some(found) =
            read_vendor_field(hive, &subkey, field, KEY_READ | KEY_WOW64_64KEY, exe_names)
        {
            return Some(found);
        }
    }
    None
}

#[cfg(windows)]
fn read_vendor_key(
    hive: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
    exe_names: &[&str],
) -> Option<PathBuf> {
    for field in ["", "InstallDir", "InstallLocation", "Path"] {
        if let Some(found) = read_vendor_field(hive, subkey, field, access, exe_names) {
            return Some(found);
        }
    }
    None
}

#[cfg(windows)]
fn read_vendor_field(
    hive: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    field: &str,
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
    exe_names: &[&str],
) -> Option<PathBuf> {
    use windows::Win32::System::Registry::{HKEY, RegCloseKey, RegOpenKeyExW};
    use windows::core::PCWSTR;

    let subkey_wide: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut key = HKEY::default();
    if unsafe {
        RegOpenKeyExW(
            hive,
            PCWSTR(subkey_wide.as_ptr()),
            Some(0),
            access,
            &mut key,
        )
    }
    .is_err()
    {
        return None;
    }
    let found = unsafe { read_reg_str(key, field) }
        .and_then(|raw| parse_registry_path_value(&raw))
        .and_then(|parsed| resolve_exe_from_path(&parsed, exe_names));
    let _ = unsafe { RegCloseKey(key) }; // ignore-ok: close registry key
    found
}

#[cfg(windows)]
fn scan_value_names_for_exe(
    hive: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    exe_names: &[&str],
) -> Option<PathBuf> {
    use windows::Win32::Foundation::ERROR_NO_MORE_ITEMS;
    use windows::Win32::System::Registry::{
        HKEY, KEY_READ, KEY_WOW64_64KEY, RegCloseKey, RegEnumValueW, RegOpenKeyExW,
    };
    use windows::core::{PCWSTR, PWSTR};

    let subkey_wide: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut key = HKEY::default();
    if unsafe {
        RegOpenKeyExW(
            hive,
            PCWSTR(subkey_wide.as_ptr()),
            Some(0),
            KEY_READ | KEY_WOW64_64KEY,
            &mut key,
        )
    }
    .is_err()
    {
        return None;
    }

    let mut found = None;
    let mut index = 0u32;
    let mut name_buf = [0u16; 512];
    loop {
        let mut name_len = name_buf.len() as u32;
        let status = unsafe {
            RegEnumValueW(
                key,
                index,
                Some(PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS || status.is_err() {
            break;
        }
        index += 1;

        let value_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
        if let Some(candidate) = extract_history_exe_path(&value_name, exe_names)
            && candidate.is_file()
        {
            found = Some(candidate);
            break;
        }
    }
    let _ = unsafe { RegCloseKey(key) }; // ignore-ok: close history registry key
    found
}
