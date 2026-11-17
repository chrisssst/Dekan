mod spawn;

pub use spawn::{spawn_as_user_child, spawn_process, spawn_with_user_token};

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, GetLastError, HANDLE, HWND, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, DuplicateTokenEx, GetTokenInformation, LUID_AND_ATTRIBUTES,
    LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, SecurityImpersonation, TOKEN_ACCESS_MASK,
    TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_PRIVILEGES, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY, TokenElevation, TokenPrimary,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    DeleteProcThreadAttributeList, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_PARENT_PROCESS, PROCESS_QUERY_INFORMATION, UpdateProcThreadAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};
use windows::core::{BOOL, PCWSTR};

use crate::cli::log;

#[link(name = "user32")]
unsafe extern "system" {
    fn ProcessIdToSessionId(dwProcessId: u32, pSessionId: *mut u32) -> BOOL;
    fn GetProcessWindowStation() -> HANDLE;
    fn GetThreadDesktop(dwThreadId: u32) -> HANDLE;
    fn GetUserObjectInformationW(
        hObj: HANDLE,
        nIndex: i32,
        pvInfo: *mut core::ffi::c_void,
        nLength: u32,
        lpnLengthNeeded: *mut u32,
    ) -> BOOL;
}

unsafe fn user_object_name(handle: HANDLE) -> Option<String> {
    if handle.0.is_null() {
        return None;
    }
    let mut buf = [0u16; 256];
    let mut needed = 0u32;
    let ok = unsafe {
        GetUserObjectInformationW(
            handle,
            2,
            buf.as_mut_ptr().cast(),
            (buf.len() * 2) as u32,
            &mut needed,
        )
    };
    if !ok.as_bool() {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub fn current_desktop_path() -> Option<String> {
    unsafe {
        let winsta = user_object_name(GetProcessWindowStation())?;
        let desktop = user_object_name(GetThreadDesktop(GetCurrentThreadId()))?;
        Some(format!("{winsta}\\{desktop}"))
    }
}

pub fn target_desktop_wide() -> Option<Vec<u16>> {
    let current = current_desktop_path()?;
    let lower = current.to_ascii_lowercase();
    if lower == "default" || lower.ends_with("\\default") {
        return None;
    }
    log(&format!(
        "[warn] current desktop is '{current}', forcing Winsta0\\Default"
    ));
    Some(to_wide("Winsta0\\Default"))
}

pub(crate) fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(crate) fn path_to_wide(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

unsafe fn for_each_process<F: FnMut(&PROCESSENTRY32W, &str) -> bool>(mut f: F) {
    unsafe {
        if let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snap, &mut entry).is_ok() {
                loop {
                    let null_pos = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let exe_name = String::from_utf16_lossy(&entry.szExeFile[..null_pos]);
                    if !f(&entry, &exe_name) || Process32NextW(snap, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap); // ignore-ok: process snapshot cleanup
        }
    }
}

pub fn is_manager_exe(name: &str) -> bool {
    name.eq_ignore_ascii_case("ltk-manager.exe") || name.eq_ignore_ascii_case("LTK Manager.exe")
}

pub fn manager_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    unsafe {
        for_each_process(|entry, name| {
            if is_manager_exe(name) {
                pids.push(entry.th32ProcessID);
            }
            true
        });
    }
    pids
}

pub fn find_new_manager_pid(before: &[u32], spawned: u32) -> Option<u32> {
    manager_pids()
        .into_iter()
        .find(|&pid| pid != spawned && !before.contains(&pid))
}

pub fn parent_pid_of(pid: u32) -> Option<u32> {
    let mut found = None;
    unsafe {
        for_each_process(|entry, _| {
            if entry.th32ProcessID == pid {
                found = Some(entry.th32ParentProcessID);
                return false;
            }
            true
        });
    }
    found
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        let _ = CloseHandle(token); // ignore-ok: own process token handle cleanup
        ok.is_ok() && elevation.TokenIsElevated != 0
    }
}

pub fn enable_privilege(name: &str) -> bool {
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
        let wide_name = to_wide(name);
        let mut luid = LUID::default();
        let mut ok = LookupPrivilegeValueW(None, PCWSTR(wide_name.as_ptr()), &mut luid).is_ok();
        if ok {
            let tp = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let adj = AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None);
            ok = adj.is_ok() && GetLastError() == ERROR_SUCCESS;
        }
        let _ = CloseHandle(token); // ignore-ok: token handle cleanup
        ok
    }
}

pub fn shell_pid() -> Option<u32> {
    unsafe {
        let shell: HWND = GetShellWindow();
        if !shell.0.is_null() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(shell, Some(&mut pid));
            if pid != 0 {
                return Some(pid);
            }
        }
        let mut current_session = 0u32;
        let _ = ProcessIdToSessionId(GetCurrentProcessId(), &mut current_session); // ignore-ok: session id query
        let mut found = None;
        for_each_process(|entry, name| {
            if name.eq_ignore_ascii_case("explorer.exe") {
                let mut other_session = 0u32;
                if ProcessIdToSessionId(entry.th32ProcessID, &mut other_session).as_bool()
                    && other_session == current_session
                {
                    found = Some(entry.th32ProcessID);
                    return false;
                }
            }
            true
        });
        found
    }
}

pub fn shell_user_token(shell_pid: u32) -> Option<HANDLE> {
    unsafe {
        let _ = enable_privilege("SeIncreaseQuotaPrivilege"); // ignore-ok: quota privilege may already be enabled
        let proc = OpenProcess(PROCESS_QUERY_INFORMATION, false, shell_pid).ok()?;
        let mut token = HANDLE::default();
        let mut primary = HANDLE::default();

        if OpenProcessToken(proc, TOKEN_DUPLICATE, &mut token).is_ok() {
            let desired = TOKEN_ACCESS_MASK(
                TOKEN_QUERY.0
                    | TOKEN_ASSIGN_PRIMARY.0
                    | TOKEN_DUPLICATE.0
                    | TOKEN_ADJUST_DEFAULT.0
                    | TOKEN_ADJUST_SESSIONID.0,
            );
            if DuplicateTokenEx(
                token,
                desired,
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut primary,
            )
            .is_err()
            {
                log(&format!("[warn] DuplicateTokenEx err={:?}", GetLastError()));
                primary = HANDLE::default();
            }
            let _ = CloseHandle(token); // ignore-ok: intermediate token handle cleanup
        } else {
            log(&format!(
                "[warn] OpenProcessToken(shell) err={:?}",
                GetLastError()
            ));
        }
        let _ = CloseHandle(proc); // ignore-ok: shell process handle cleanup

        if !primary.0.is_null() {
            Some(primary)
        } else {
            None
        }
    }
}

pub struct ProcThreadAttributeListGuard {
    _buffer: Vec<u8>,
    _parent_handle: Box<HANDLE>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl ProcThreadAttributeListGuard {
    pub fn new_parent(parent_handle: HANDLE) -> Option<Self> {
        unsafe {
            let mut size = 0usize;
            let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size); // ignore-ok: size inquiry returns failure
            let mut buffer = vec![0u8; size];
            let boxed_handle = Box::new(parent_handle);
            let list = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr() as *mut _);
            if InitializeProcThreadAttributeList(Some(list), 1, None, &mut size).is_err() {
                let _ = CloseHandle(*boxed_handle); // ignore-ok: cleanup parent handle on init failure
                return None;
            }
            let handle_ptr = &*boxed_handle as *const HANDLE as *const core::ffi::c_void;
            if UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize,
                Some(handle_ptr),
                std::mem::size_of::<HANDLE>(),
                None,
                None,
            )
            .is_err()
            {
                DeleteProcThreadAttributeList(list);
                let _ = CloseHandle(*boxed_handle); // ignore-ok: cleanup parent handle on update failure
                return None;
            }
            Some(Self {
                _buffer: buffer,
                _parent_handle: boxed_handle,
                list,
            })
        }
    }

    pub fn as_ptr(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.list
    }
}

impl Drop for ProcThreadAttributeListGuard {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.list);
            let _ = CloseHandle(*self._parent_handle); // ignore-ok: parent handle cleanup after attribute list delete
        }
    }
}
