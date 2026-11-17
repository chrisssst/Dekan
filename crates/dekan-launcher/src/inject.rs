use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, LPTHREAD_START_ROUTINE, OpenProcess,
    PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ,
    PROCESS_VM_WRITE, WaitForSingleObject,
};
use windows::core::{s, w};

use crate::cli::log;

pub fn resolve_dll_path() -> Option<PathBuf> {
    let self_exe = std::env::current_exe().ok()?;
    let parent = self_exe.parent()?;
    let candidate = parent.join("dekan_bridge.dll");
    if candidate.is_file() {
        log(&format!(
            "[resolve] dekan_bridge.dll = {}",
            candidate.display()
        ));
        Some(candidate)
    } else {
        log(&format!(
            "[err] dekan_bridge.dll not found: {}",
            candidate.display()
        ));
        None
    }
}

pub fn resolve_default_exe() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(self_exe) = std::env::current_exe() {
        if let Some(parent) = self_exe.parent() {
            candidates.push(parent.join("ltk-manager.exe"));
            candidates.push(parent.join("LTK Manager.exe"));
        }
    }

    candidates.push(PathBuf::from(
        r"C:\Program Files\LTK Manager\ltk-manager.exe",
    ));
    candidates.push(PathBuf::from(
        r"C:\Program Files\LTK Manager\LTK Manager.exe",
    ));

    if let Ok(program_files) = std::env::var("ProgramFiles") {
        candidates.push(
            PathBuf::from(program_files)
                .join("LTK Manager")
                .join("ltk-manager.exe"),
        );
    }
    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local_app)
                .join("Programs")
                .join("LTK Manager")
                .join("ltk-manager.exe"),
        );
    }

    for candidate in candidates {
        if candidate.is_file() {
            log(&format!(
                "[resolve] found default exe: {}",
                candidate.display()
            ));
            return Some(candidate);
        }
    }

    log("[resolve] no candidate found");
    None
}

pub fn inject_dll(pid: u32, dll_path: &Path) -> bool {
    unsafe {
        let process_access = PROCESS_CREATE_THREAD
            | PROCESS_QUERY_INFORMATION
            | PROCESS_VM_OPERATION
            | PROCESS_VM_WRITE
            | PROCESS_VM_READ;

        let proc = match OpenProcess(process_access, false, pid) {
            Ok(p) => p,
            Err(e) => {
                log(&format!("[err] OpenProcess(pid={pid}) err={e}"));
                return false;
            }
        };

        let wide_path: Vec<u16> = dll_path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let bytes = wide_path.len() * std::mem::size_of::<u16>();

        let remote = VirtualAllocEx(proc, None, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if remote.is_null() {
            log("[err] VirtualAllocEx failed");
            let _ = CloseHandle(proc); // ignore-ok: process handle released on allocation failure
            return false;
        }

        if let Err(e) = WriteProcessMemory(
            proc,
            remote,
            wide_path.as_ptr() as *const core::ffi::c_void,
            bytes,
            None,
        ) {
            log(&format!("[err] WriteProcessMemory err={e}"));
            let _ = VirtualFreeEx(proc, remote, 0, MEM_RELEASE); // ignore-ok: cleanup allocated memory
            let _ = CloseHandle(proc); // ignore-ok: process handle cleanup
            return false;
        }

        let kernel32 = match GetModuleHandleW(w!("kernel32.dll")) {
            Ok(m) => m,
            Err(_) => {
                log("[err] GetModuleHandleW(kernel32.dll) failed");
                let _ = VirtualFreeEx(proc, remote, 0, MEM_RELEASE); // ignore-ok: cleanup allocated memory
                let _ = CloseHandle(proc); // ignore-ok: process handle cleanup
                return false;
            }
        };

        let load_lib_proc = match GetProcAddress(kernel32, s!("LoadLibraryW")) {
            Some(p) => p,
            None => {
                log("[err] GetProcAddress(LoadLibraryW) failed");
                let _ = VirtualFreeEx(proc, remote, 0, MEM_RELEASE); // ignore-ok: cleanup allocated memory
                let _ = CloseHandle(proc); // ignore-ok: process handle cleanup
                return false;
            }
        };

        let start_routine: LPTHREAD_START_ROUTINE = std::mem::transmute(load_lib_proc);

        let thread = match CreateRemoteThread(proc, None, 0, start_routine, Some(remote), 0, None) {
            Ok(th) => th,
            Err(e) => {
                log(&format!("[err] CreateRemoteThread err={e}"));
                let _ = VirtualFreeEx(proc, remote, 0, MEM_RELEASE); // ignore-ok: cleanup allocated memory
                let _ = CloseHandle(proc); // ignore-ok: process handle cleanup
                return false;
            }
        };

        let wait_result = WaitForSingleObject(thread, 15000);
        let mut exit_code = 0u32;
        if wait_result != WAIT_TIMEOUT {
            let _ = GetExitCodeThread(thread, &mut exit_code); // ignore-ok: thread exit code read
            log(&format!(
                "[inject] LoadLibraryW(pid={pid}) -> 0x{exit_code:X}"
            ));
        } else {
            log("[warn] inject timed out");
            exit_code = 1;
        }

        let _ = CloseHandle(thread); // ignore-ok: thread handle cleanup
        let _ = VirtualFreeEx(proc, remote, 0, MEM_RELEASE); // ignore-ok: remote memory cleanup
        let _ = CloseHandle(proc); // ignore-ok: process handle cleanup

        exit_code != 0
    }
}
