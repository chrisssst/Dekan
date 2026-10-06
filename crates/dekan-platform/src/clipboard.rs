use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::error::PlatformError;

const MAX_READ_CHARS: usize = 4096;

struct OpenGuard;

impl OpenGuard {
    fn open() -> Result<Self, PlatformError> {
        unsafe { OpenClipboard(None) }
            .map_err(|e| PlatformError::Window(format!("clipboard unavailable: {e}")))?;
        Ok(Self)
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        // ignore-ok: nothing can be done if closing fails; the next open reports it
        let _ = unsafe { CloseClipboard() };
    }
}

pub fn set_text(text: &str) -> Result<(), PlatformError> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * std::mem::size_of::<u16>();

    let _guard = OpenGuard::open()?;

    unsafe { EmptyClipboard() }
        .map_err(|e| PlatformError::Window(format!("clipboard could not be emptied: {e}")))?;

    unsafe {
        let block = GlobalAlloc(GMEM_MOVEABLE, bytes)
            .map_err(|e| PlatformError::Window(format!("clipboard allocation failed: {e}")))?;
        let target = GlobalLock(block) as *mut u16;
        if target.is_null() {
            // ignore-ok: freeing our own block after a failure already being reported
            let _ = GlobalFree(block);
            return Err(PlatformError::Window(
                "clipboard memory could not be locked".into(),
            ));
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());

        // ignore-ok: GlobalUnlock reports "not locked any more" as an error on success
        let _ = GlobalUnlock(block);

        if let Err(e) = SetClipboardData(u32::from(CF_UNICODETEXT.0), HANDLE(block.0)) {
            // ignore-ok: the clipboard refused the block, so it is still ours to free
            let _ = GlobalFree(block);
            return Err(PlatformError::Window(format!(
                "clipboard refused the text: {e}"
            )));
        }
    }
    Ok(())
}

pub fn get_text() -> Result<Option<String>, PlatformError> {
    let _guard = OpenGuard::open()?;

    unsafe {
        let Ok(handle) = GetClipboardData(u32::from(CF_UNICODETEXT.0)) else {
            return Ok(None);
        };
        let block = HGLOBAL(handle.0);
        let source = GlobalLock(block) as *const u16;
        if source.is_null() {
            return Ok(None);
        }
        let mut len = 0usize;
        while len < MAX_READ_CHARS && *source.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(source, len));

        // ignore-ok: see set_text — the unlock result carries no failure worth reporting
        let _ = GlobalUnlock(block);
        Ok(Some(text))
    }
}
