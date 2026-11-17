use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, PROCESS_VM_READ,
    TerminateProcess,
};
use windows::core::BOOL;

#[cfg(target_arch = "x86_64")]
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(
        process_handle: HANDLE,
        info_class: u32,
        process_information: *mut core::ffi::c_void,
        length: u32,
        return_length: *mut u32,
    ) -> i32;
}

#[cfg(target_arch = "x86_64")]
unsafe extern "system" {
    fn ReadProcessMemory(
        process: HANDLE,
        base_address: *const core::ffi::c_void,
        buffer: *mut core::ffi::c_void,
        size: usize,
        bytes_read: *mut usize,
    ) -> BOOL;
}

#[must_use]
pub fn is_default_desktop(desktop: &str) -> bool {
    let trimmed = desktop.trim();
    trimmed.eq_ignore_ascii_case("Default") || trimmed.to_ascii_lowercase().ends_with("\\default")
}

#[must_use]
#[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]
pub fn process_desktop(pid: u32) -> Option<String> {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        )
        .ok()?;
        let desktop = read_peb_desktop_info(handle);
        let _ = CloseHandle(handle); // ignore-ok: process handle released after PEB read
        desktop
    }
    #[cfg(not(target_arch = "x86_64"))]
    None
}

#[must_use]
pub fn terminate_process(pid: u32) -> bool {
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }) else {
        return false;
    };
    let ok = unsafe { TerminateProcess(handle, 1) }.is_ok();
    let _ = unsafe { CloseHandle(handle) }; // ignore-ok: process handle released after terminate
    ok
}

#[cfg(target_arch = "x86_64")]
unsafe fn read_peb_desktop_info(handle: HANDLE) -> Option<String> {
    #[repr(C)]
    #[derive(Default)]
    struct ProcessBasicInfo {
        reserved1: usize,
        peb_base_address: usize,
        reserved2: [usize; 2],
        unique_process_id: usize,
        reserved3: usize,
    }

    #[repr(C)]
    #[derive(Default)]
    struct UnicodeStr {
        length: u16,
        maximum_length: u16,
        buffer: u64,
    }

    let mut pbi = ProcessBasicInfo::default();
    let mut ret_len = 0u32;
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            0,
            (&mut pbi as *mut ProcessBasicInfo).cast(),
            std::mem::size_of::<ProcessBasicInfo>() as u32,
            &mut ret_len,
        )
    };
    if status != 0 || pbi.peb_base_address == 0 {
        return None;
    }

    let mut pp_addr = 0u64;
    let mut read = 0usize;
    if !unsafe {
        ReadProcessMemory(
            handle,
            (pbi.peb_base_address + 0x20) as *const core::ffi::c_void,
            (&mut pp_addr as *mut u64).cast(),
            std::mem::size_of::<u64>(),
            &mut read,
        )
    }
    .as_bool()
        || read != std::mem::size_of::<u64>()
        || pp_addr == 0
    {
        return None;
    }

    let mut ustr = UnicodeStr::default();
    if !unsafe {
        ReadProcessMemory(
            handle,
            (pp_addr as usize + 0xc0) as *const core::ffi::c_void,
            (&mut ustr as *mut UnicodeStr).cast(),
            std::mem::size_of::<UnicodeStr>(),
            &mut read,
        )
    }
    .as_bool()
        || read != std::mem::size_of::<UnicodeStr>()
        || ustr.length == 0
        || ustr.buffer == 0
        || ustr.length > 1024
    {
        return None;
    }

    let mut wide = vec![0u16; usize::from(ustr.length) / 2];
    let byte_len = wide.len() * 2;
    if !unsafe {
        ReadProcessMemory(
            handle,
            ustr.buffer as usize as *const core::ffi::c_void,
            wide.as_mut_ptr().cast(),
            byte_len,
            &mut read,
        )
    }
    .as_bool()
        || read != byte_len
    {
        return None;
    }

    Some(String::from_utf16_lossy(&wide))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_own_desktop_and_identifies_default() {
        let own = process_desktop(std::process::id());
        assert!(own.is_some(), "expected own process desktop");
        assert!(is_default_desktop(r"WinSta0\Default"));
        assert!(is_default_desktop("Default"));
        assert!(!is_default_desktop("exebox-2NAFTZ75DP3HFLWMMTVA6DPWVL"));
    }
}
