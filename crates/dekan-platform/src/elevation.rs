use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_elevated_does_not_panic() {
        let _ = is_elevated(); // ignore-ok: test execution without side effects
    }
}
