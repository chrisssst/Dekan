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
