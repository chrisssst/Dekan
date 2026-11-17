use std::path::Path;

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
    if !found.as_bool()
        || info_ptr.is_null()
        || (info_len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
    {
        return None;
    }

    let fixed = unsafe { &*(info_ptr.cast::<VS_FIXEDFILEINFO>()) };
    let major = ((fixed.dwFileVersionMS >> 16) & 0xFFFF) as u16;
    let minor = (fixed.dwFileVersionMS & 0xFFFF) as u16;
    let patch = ((fixed.dwFileVersionLS >> 16) & 0xFFFF) as u16;
    Some((major, minor, patch))
}

#[cfg(not(windows))]
#[must_use]
pub fn read_pe_version(_path: &Path) -> Option<(u16, u16, u16)> {
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
}
