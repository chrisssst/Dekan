use std::path::Path;

use tracing::warn;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use crate::error::PlatformError;

pub use super::is_web_page;

pub fn open_folder(dir: &Path) -> Result<(), PlatformError> {
    if !dir.is_dir() {
        return Err(PlatformError::Path(format!(
            "'{}' is not an existing folder",
            dir.display()
        )));
    }

    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(dir.as_os_str()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };

    let code = result.0 as isize;
    if code <= 32 {
        return Err(PlatformError::Window(format!(
            "ShellExecuteW could not open '{}' (code {code})",
            dir.display()
        )));
    }
    Ok(())
}

pub fn open_web_page(url: &str) -> Result<(), PlatformError> {
    if !is_web_page(url) {
        return Err(PlatformError::Path(format!("'{url}' is not an https page")));
    }

    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };

    let code = result.0 as isize;
    if code <= 32 {
        return Err(PlatformError::Window(format!(
            "ShellExecuteW could not open '{url}' (code {code})"
        )));
    }
    Ok(())
}

pub fn message_box(title: &str, text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
    };

    let answer = unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };
    note_if_not_shown(answer.0, title);
}

pub(crate) fn note_if_not_shown(answer: i32, title: &str) -> bool {
    if answer == 0 {
        warn!(
            title,
            error = %windows::core::Error::from_thread(),
            "Windows could not show a message box"
        );
        return false;
    }
    true
}

pub fn message_box_warning(title: &str, text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
    };

    let answer = unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_OK | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };
    note_if_not_shown(answer.0, title);
}

#[must_use]
pub fn message_box_question(title: &str, text: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MessageBoxW,
    };

    let answer = unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };
    note_if_not_shown(answer.0, title) && answer == IDYES
}

pub fn run_elevated_wait(exe: &Path, args: &str) -> Result<bool, PlatformError> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
    use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    use windows::core::PCWSTR;

    let file_h = HSTRING::from(exe.as_os_str());
    let params_h = HSTRING::from(args);
    let verb = if crate::elevation::is_elevated() {
        w!("open")
    } else {
        w!("runas")
    };
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb,
        lpFile: PCWSTR(file_h.as_ptr()),
        lpParameters: PCWSTR(params_h.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    let res = unsafe { ShellExecuteExW(&mut info) };
    if let Err(e) = res {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            return Ok(false);
        }
        return Err(PlatformError::Window(format!(
            "ShellExecuteExW(runas) failed for '{}': {e}",
            exe.display()
        )));
    }
    if info.hProcess.is_invalid() {
        return Ok(false);
    }
    unsafe {
        let _ = WaitForSingleObject(info.hProcess, INFINITE); // ignore-ok: wait for elevated helper exit
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code); // ignore-ok: read helper exit code
        let _ = CloseHandle(info.hProcess); // ignore-ok: release process handle
        Ok(code == 0)
    }
}

#[must_use]
pub fn read_hklm_string(subkey: &str, value_name: &str) -> Option<String> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
        RegCloseKey, RegGetValueW, RegOpenKeyExW,
    };
    use windows::core::PCWSTR;

    let sub_h = HSTRING::from(subkey);
    let val_h = HSTRING::from(value_name);
    let mut key = HKEY::default();
    if unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(sub_h.as_ptr()),
            Some(0),
            KEY_READ | KEY_WOW64_64KEY,
            &mut key,
        )
    }
    .is_err()
    {
        return None;
    }
    let mut buf = vec![0u16; 512];
    let mut bytes = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR(val_h.as_ptr()),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    }
    .is_ok();
    let _ = unsafe { RegCloseKey(key) }; // ignore-ok: close registry key
    if !ok {
        return None;
    }
    let chars = (bytes as usize / 2).min(buf.len());
    let slice = &buf[..chars];
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    Some(String::from_utf16_lossy(&slice[..end]))
}

pub fn dismiss_question(title: &str) -> bool {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetWindowThreadProcessId, IDNO, PostMessageW, WM_COMMAND,
    };

    unsafe {
        let Ok(dialog) = FindWindowW(w!("#32770"), &HSTRING::from(title)) else {
            return false;
        };
        let mut owner = 0u32;
        GetWindowThreadProcessId(dialog, Some(&mut owner));
        owner == GetCurrentProcessId()
            && PostMessageW(Some(dialog), WM_COMMAND, WPARAM(IDNO.0 as usize), LPARAM(0)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_https_pages_are_opened() {
        assert!(is_web_page("https://github.com/owner/repo/releases/latest"));
        for url in [
            "",
            "https://",
            "https:///etc",
            "http://github.com",
            "file:///C:/Windows/System32/cmd.exe",
            r"C:\Windows\System32\cmd.exe",
            "https://example.com/a b",
            r#"https://example.com/"x"#,
            r"https://example.com\x",
            "ms-settings:",
        ] {
            assert!(!is_web_page(url), "{url}");
        }
    }
}
