use windows::Win32::Foundation::{
    CloseHandle, ERROR_NOT_ALL_ASSIGNED, ERROR_SUCCESS, GetLastError, HANDLE, LUID, SetLastError,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
    TokenElevation,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut return_length = 0u32;
        let success = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        );

        let _ = CloseHandle(token); // ignore-ok: token handle released

        success.is_ok() && elevation.TokenIsElevated != 0
    }
}

pub fn enable_debug_privilege() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .is_err()
        {
            return false;
        }

        let mut luid = LUID::default();
        let privilege_name: Vec<u16> = "SeDebugPrivilege\0".encode_utf16().collect();
        let lookup_ok =
            LookupPrivilegeValueW(None, PCWSTR(privilege_name.as_ptr()), &mut luid).is_ok();

        if !lookup_ok {
            let _ = CloseHandle(token); // ignore-ok: token handle released
            return false;
        }

        let new_state = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        SetLastError(ERROR_SUCCESS);
        let adjust_ok = AdjustTokenPrivileges(token, false, Some(&new_state), 0, None, None)
            .is_ok()
            && GetLastError() != ERROR_NOT_ALL_ASSIGNED;
        let _ = CloseHandle(token); // ignore-ok: token handle released
        adjust_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_elevated_does_not_panic() {
        let _ = is_elevated(); // ignore-ok: test execution without side effects
    }

    #[test]
    fn test_enable_debug_privilege_does_not_panic() {
        let _ = enable_debug_privilege(); // ignore-ok: test execution without side effects
    }
}
