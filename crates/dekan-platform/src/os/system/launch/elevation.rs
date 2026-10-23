use std::path::Path;

use tracing::debug;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HRESULT, HSTRING, PCWSTR, w};

use crate::error::PlatformError;

const ELEVATED_WAIT_MS: u32 = 120_000;

pub use super::ElevatedRun;

pub fn run_elevated(exe: &Path, parameters: &str) -> Result<ElevatedRun, PlatformError> {
    let file = HSTRING::from(exe.as_os_str());
    let parameters = HSTRING::from(parameters);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
            return Ok(ElevatedRun::Declined);
        }
        return Err(e.into());
    }
    let waited = unsafe { WaitForSingleObject(info.hProcess, ELEVATED_WAIT_MS) };
    let result = if waited == WAIT_OBJECT_0 {
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(info.hProcess, &mut code) }
            .map(|()| code)
            .map_err(PlatformError::from)
    } else if waited == WAIT_TIMEOUT {
        Err(PlatformError::Io {
            context: format!(
                "the elevated process did not finish within {} s",
                ELEVATED_WAIT_MS / 1000
            ),
            source: std::io::ErrorKind::TimedOut.into(),
        })
    } else {
        Err(PlatformError::Win32(windows::core::Error::from_thread()))
    };
    let _ = unsafe { CloseHandle(info.hProcess) }; // ignore-ok: the handle is released whether the elevated process finished or not
    result.map(ElevatedRun::Finished)
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if let Err(e) = OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) {
            debug!(error = %e, "Own process token unavailable; assuming not elevated");
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

        if let Err(e) = &success {
            debug!(error = %e, "Token elevation unreadable; assuming not elevated");
        }
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
