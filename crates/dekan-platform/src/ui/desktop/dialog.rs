use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
    OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::core::{PCWSTR, PWSTR};

use crate::error::PlatformError;

const PATH_BUFFER: usize = 32 * 1024;

pub fn pick_file(
    owner: isize,
    title: &str,
    filter_label: &str,
    patterns: &str,
) -> Result<Option<PathBuf>, PlatformError> {
    let wide = |s: &str| s.encode_utf16().collect::<Vec<u16>>();

    let mut filter = wide(filter_label);
    filter.push(0);
    filter.extend(wide(patterns));
    filter.extend([0, 0]);
    let mut title_w = wide(title);
    title_w.push(0);
    let mut buffer = vec![0u16; PATH_BUFFER];

    let mut request = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND(owner as *mut _),
        lpstrFilter: PCWSTR(filter.as_ptr()),
        nFilterIndex: 1,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: PATH_BUFFER as u32,
        lpstrTitle: PCWSTR(title_w.as_ptr()),
        Flags: OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_HIDEREADONLY,
        ..Default::default()
    };

    let chosen = unsafe { GetOpenFileNameW(&mut request) }.as_bool();
    if !chosen {
        let code = unsafe { CommDlgExtendedError() };
        if code.0 == 0 {
            return Ok(None);
        }
        return Err(PlatformError::Window(format!(
            "file dialog failed (CommDlgExtendedError {:#x})",
            code.0
        )));
    }

    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Ok(Some(PathBuf::from(OsString::from_wide(&buffer[..len]))))
}
