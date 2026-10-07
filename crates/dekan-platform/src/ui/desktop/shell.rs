use std::path::Path;

use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use crate::error::PlatformError;

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

#[must_use]
pub fn is_web_page(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty()
            && !rest.starts_with('/')
            && url
                .chars()
                .all(|c| c.is_ascii_graphic() && c != '"' && c != '\\')
    })
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

    unsafe {
        // ignore-ok: an informational box has one button; which one came back carries nothing
        let _ = MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND,
        );
    }
}

pub fn message_box_warning(title: &str, text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
    };

    unsafe {
        // ignore-ok: an informational box has one button; which one came back carries nothing
        let _ = MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_OK | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND,
        );
    }
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
    answer == IDYES
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
