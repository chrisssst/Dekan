use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    EqualSid, GetTokenInformation, TOKEN_IMPERSONATE, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::RemoteDesktop::{ProcessIdToSessionId, WTSGetActiveConsoleSessionId};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::error::PlatformError;

const NO_CONSOLE_SESSION: u32 = 0xFFFF_FFFF;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    SameUser { path: PathBuf },

    DesktopUser { path: PathBuf, own: Option<PathBuf> },

    OwnProfile { path: PathBuf, reason: String },

    Unresolved { reason: String },
}

impl Resolution {
    fn path(&self) -> Option<&PathBuf> {
        match self {
            Self::SameUser { path }
            | Self::DesktopUser { path, .. }
            | Self::OwnProfile { path, .. } => Some(path),
            Self::Unresolved { .. } => None,
        }
    }
}

static RESOLUTION: OnceLock<Resolution> = OnceLock::new();

pub fn resolution() -> &'static Resolution {
    RESOLUTION.get_or_init(resolve)
}

/// The desktop user's `%LOCALAPPDATA%`.
pub fn local_app_data() -> Result<PathBuf, PlatformError> {
    match resolution() {
        Resolution::Unresolved { reason } => Err(PlatformError::Path(format!(
            "LOCALAPPDATA could not be resolved: {reason}"
        ))),
        resolved => resolved
            .path()
            .cloned()
            .ok_or_else(|| PlatformError::Path("LOCALAPPDATA could not be resolved".into())),
    }
}

fn resolve() -> Resolution {
    let own = own_local_app_data();
    match desktop_user_local_app_data() {
        Ok(DesktopLookup::SameUser) => match own {
            Some(path) => Resolution::SameUser { path },
            None => Resolution::Unresolved {
                reason: "the process's own profile has no LocalAppData".into(),
            },
        },
        Ok(DesktopLookup::OtherUser(path)) => Resolution::DesktopUser { path, own },
        Ok(DesktopLookup::NotFound(reason)) | Err(reason) => match own {
            Some(path) => Resolution::OwnProfile { path, reason },
            None => Resolution::Unresolved { reason },
        },
    }
}

enum DesktopLookup {
    SameUser,
    OtherUser(PathBuf),
    NotFound(String),
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) }; // ignore-ok: nothing to recover from a failed close
        }
    }
}

fn own_local_app_data() -> Option<PathBuf> {
    known_local_app_data(None).or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
}

fn known_local_app_data(token: Option<HANDLE>) -> Option<PathBuf> {
    unsafe {
        let pwstr = SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            KF_FLAG_DEFAULT,
            token.unwrap_or_default(),
        )
        .ok()?;
        let path = pwstr.as_wide().to_vec();
        CoTaskMemFree(Some(pwstr.0 as *const _));
        if path.is_empty() {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&path)))
    }
}

fn token_user(token: HANDLE) -> Result<Vec<u8>, String> {
    unsafe {
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed); // ignore-ok: the size probe always "fails" with ERROR_INSUFFICIENT_BUFFER
        if needed == 0 {
            return Err("token user size unavailable".into());
        }
        let mut buffer = vec![0u8; needed as usize];
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .map_err(|e| format!("token user unreadable: {e}"))?;
        Ok(buffer)
    }
}

fn same_user(a: &[u8], b: &[u8]) -> bool {
    unsafe {
        let a = &*(a.as_ptr().cast::<TOKEN_USER>());
        let b = &*(b.as_ptr().cast::<TOKEN_USER>());
        EqualSid(a.User.Sid, b.User.Sid).is_ok()
    }
}

fn explorer_pids() -> Result<Vec<u32>, String> {
    unsafe {
        let snapshot = OwnedHandle(
            CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
                .map_err(|e| format!("process snapshot failed: {e}"))?,
        );
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut pids = Vec::new();
        let mut more = Process32FirstW(snapshot.0, &mut entry).is_ok();
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = OsString::from_wide(&entry.szExeFile[..len]);
            if name
                .to_str()
                .is_some_and(|n| n.eq_ignore_ascii_case("explorer.exe"))
            {
                pids.push(entry.th32ProcessID);
            }
            more = Process32NextW(snapshot.0, &mut entry).is_ok();
        }
        Ok(pids)
    }
}

fn desktop_user_local_app_data() -> Result<DesktopLookup, String> {
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    if console == NO_CONSOLE_SESSION {
        return Ok(DesktopLookup::NotFound(
            "no session is attached to the console".into(),
        ));
    }

    let own_user = unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| format!("own token unavailable: {e}"))?;
        let token = OwnedHandle(token);
        token_user(token.0)?
    };

    let mut last_error = String::from("no explorer.exe in the console session");
    for pid in explorer_pids()? {
        let mut session = 0u32;

        if unsafe { ProcessIdToSessionId(pid, &mut session) }.is_err() || session != console {
            continue;
        }

        let found = unsafe {
            let process = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(process) => OwnedHandle(process),
                Err(e) => {
                    last_error = format!("explorer.exe (pid {pid}) not openable: {e}");
                    continue;
                }
            };
            let mut token = HANDLE::default();
            if let Err(e) = OpenProcessToken(process.0, TOKEN_QUERY | TOKEN_IMPERSONATE, &mut token)
            {
                last_error = format!("explorer.exe (pid {pid}) token not openable: {e}");
                continue;
            }
            let token = OwnedHandle(token);
            let desktop_user = match token_user(token.0) {
                Ok(user) => user,
                Err(e) => {
                    last_error = e;
                    continue;
                }
            };
            if same_user(&own_user, &desktop_user) {
                return Ok(DesktopLookup::SameUser);
            }
            known_local_app_data(Some(token.0))
        };
        match found {
            Some(path) => return Ok(DesktopLookup::OtherUser(path)),
            None => last_error = format!("explorer.exe (pid {pid}) user has no LocalAppData"),
        }
    }
    Ok(DesktopLookup::NotFound(last_error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_folder_resolves_to_an_existing_directory() {
        match resolution() {
            Resolution::Unresolved { reason } => panic!("unresolved: {reason}"),
            resolved => {
                let path = resolved.path().expect("resolved path");
                assert!(path.is_dir(), "{} is not a directory", path.display());
            }
        }
        assert!(local_app_data().is_ok());
    }

    #[test]
    fn the_own_profile_matches_the_environment() {
        if let (Some(known), Some(env)) = (
            known_local_app_data(None),
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        ) {
            assert_eq!(
                known.to_string_lossy().to_lowercase(),
                env.to_string_lossy().to_lowercase()
            );
        }
    }
}
