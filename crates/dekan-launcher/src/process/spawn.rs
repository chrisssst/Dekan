use std::path::Path;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ELEVATION_REQUIRED, GetLastError, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    CREATE_PROCESS_LOGON_FLAGS, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, CreateProcessW,
    CreateProcessWithTokenW, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    STARTF_USESHOWWINDOW, STARTUPINFOEXW, STARTUPINFOW, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{PCWSTR, PWSTR};

use super::{enable_privilege, path_to_wide, target_desktop_wide, to_wide};
use crate::cli::log;
use crate::env::with_env_var;

pub fn spawn_as_user_child(
    token: HANDLE,
    env: &[u16],
    exe_path: &Path,
    no_hide: bool,
    log_dir: Option<&Path>,
) -> Option<u32> {
    let self_exe = std::env::current_exe().ok()?;
    let pid_file = std::env::temp_dir().join(format!("dekan_launcher_{}.pid", std::process::id()));
    let _ = std::fs::remove_file(&pid_file); // ignore-ok: stale pid file cleanup

    let mut cmd_string = format!(
        "\"{}\" --as-user-child --quiet --exe \"{}\" --pid-file \"{}\"",
        self_exe.display(),
        exe_path.display(),
        pid_file.display()
    );
    if no_hide {
        cmd_string.push_str(" --no-hide");
    }
    if let Some(dir) = log_dir {
        cmd_string.push_str(&format!(" --log-dir \"{}\"", dir.display()));
    }

    let helper_env = with_env_var(env, "__COMPAT_LAYER", "RunAsInvoker");
    let mut desktop_wide = target_desktop_wide();
    let mut si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    if let Some(ref mut desk) = desktop_wide {
        si.lpDesktop = PWSTR(desk.as_mut_ptr());
    }
    if !no_hide {
        si.dwFlags = STARTF_USESHOWWINDOW;
        si.wShowWindow = SW_HIDE.0 as u16;
    }

    let mut pi = PROCESS_INFORMATION::default();
    let mut cmd_wide = to_wide(&cmd_string);
    let self_wide = path_to_wide(&self_exe);

    unsafe {
        let spawn_res = CreateProcessWithTokenW(
            token,
            CREATE_PROCESS_LOGON_FLAGS(0),
            PCWSTR(self_wide.as_ptr()),
            Some(PWSTR(cmd_wide.as_mut_ptr())),
            PROCESS_CREATION_FLAGS(CREATE_UNICODE_ENVIRONMENT.0),
            Some(helper_env.as_ptr() as *const core::ffi::c_void),
            PCWSTR::null(),
            &si,
            &mut pi,
        );

        if let Err(e) = spawn_res {
            let err = GetLastError();
            log(&format!("[warn] helper spawn err={err:?} ({e})"));
            if err == ERROR_ELEVATION_REQUIRED {
                log(
                    "[warn] dekan_launcher.exe is set to run as administrator in compatibility settings",
                );
            }
            return None;
        }

        log(&format!("[launch] helper started pid={}", pi.dwProcessId));
        let mut result_pid = None;
        if WaitForSingleObject(pi.hProcess, 45000) == WAIT_OBJECT_0 {
            let mut code = 1u32;
            let _ = GetExitCodeProcess(pi.hProcess, &mut code); // ignore-ok: exit code read
            if code == 0 {
                if let Ok(text) = std::fs::read_to_string(&pid_file) {
                    if let Ok(parsed) = text.trim().parse::<u32>() {
                        result_pid = Some(parsed);
                    }
                }
            } else {
                log(&format!("[warn] helper exited with code {code}"));
            }
        } else {
            log("[warn] helper did not finish in time");
        }

        let _ = std::fs::remove_file(&pid_file); // ignore-ok: remove temp pid file
        let _ = CloseHandle(pi.hThread); // ignore-ok: helper thread handle cleanup
        let _ = CloseHandle(pi.hProcess); // ignore-ok: helper process handle cleanup
        result_pid
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_with_user_token(
    token: HANDLE,
    env: &[u16],
    exe_path: &Path,
    cmd_line: &str,
    work_dir: Option<&Path>,
    attr_list: Option<LPPROC_THREAD_ATTRIBUTE_LIST>,
    flags: PROCESS_CREATION_FLAGS,
    no_hide: bool,
) -> Result<PROCESS_INFORMATION, windows::core::Error> {
    unsafe {
        let _ = enable_privilege("SeAssignPrimaryTokenPrivilege"); // ignore-ok: enable primary token privilege
        let _ = enable_privilege("SeIncreaseQuotaPrivilege"); // ignore-ok: enable quota privilege

        let mut desktop_wide = target_desktop_wide();
        let mut six = STARTUPINFOEXW::default();
        let (f, cb) = if let Some(list) = attr_list {
            six.lpAttributeList = list;
            (
                flags | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                std::mem::size_of::<STARTUPINFOEXW>() as u32,
            )
        } else {
            (
                flags | CREATE_UNICODE_ENVIRONMENT,
                std::mem::size_of::<STARTUPINFOW>() as u32,
            )
        };
        six.StartupInfo.cb = cb;
        if let Some(ref mut desk) = desktop_wide {
            six.StartupInfo.lpDesktop = PWSTR(desk.as_mut_ptr());
        }
        if !no_hide {
            six.StartupInfo.dwFlags = STARTF_USESHOWWINDOW;
            six.StartupInfo.wShowWindow = SW_HIDE.0 as u16;
        }

        let mut pi = PROCESS_INFORMATION::default();
        let mut cmd_wide = to_wide(cmd_line);
        let exe_wide = path_to_wide(exe_path);
        let work_wide = work_dir.map(path_to_wide);

        CreateProcessAsUserW(
            Some(token),
            PCWSTR(exe_wide.as_ptr()),
            Some(PWSTR(cmd_wide.as_mut_ptr())),
            None,
            None,
            false,
            f,
            Some(env.as_ptr() as *const core::ffi::c_void),
            work_wide
                .as_ref()
                .map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr())),
            &six.StartupInfo,
            &mut pi,
        )?;
        Ok(pi)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_process(
    token: Option<HANDLE>,
    env: Option<&[u16]>,
    exe_path: &Path,
    cmd_line: &str,
    work_dir: Option<&Path>,
    attr_list: Option<LPPROC_THREAD_ATTRIBUTE_LIST>,
    flags: PROCESS_CREATION_FLAGS,
    no_hide: bool,
) -> Result<PROCESS_INFORMATION, windows::core::Error> {
    unsafe {
        let mut desktop_wide = target_desktop_wide();
        let mut six = STARTUPINFOEXW::default();
        let with_attr = attr_list.is_some();
        let f = if with_attr {
            flags | EXTENDED_STARTUPINFO_PRESENT
        } else {
            flags
        };
        six.StartupInfo.cb = if with_attr {
            std::mem::size_of::<STARTUPINFOEXW>() as u32
        } else {
            std::mem::size_of::<STARTUPINFOW>() as u32
        };
        if let Some(ref mut desk) = desktop_wide {
            six.StartupInfo.lpDesktop = PWSTR(desk.as_mut_ptr());
        }
        if let Some(list) = attr_list {
            six.lpAttributeList = list;
        }
        if !no_hide {
            six.StartupInfo.dwFlags = STARTF_USESHOWWINDOW;
            six.StartupInfo.wShowWindow = SW_HIDE.0 as u16;
        }

        let mut pi = PROCESS_INFORMATION::default();
        let mut cmd_wide = to_wide(cmd_line);
        let exe_wide = path_to_wide(exe_path);
        let work_wide = work_dir.map(path_to_wide);
        let work_pcwstr = work_wide
            .as_ref()
            .map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr()));

        let mut invoke_spawn =
            |use_attr: bool,
             flags_to_use: PROCESS_CREATION_FLAGS,
             proc_info: &mut PROCESS_INFORMATION| {
                let mut si_copy = six;
                si_copy.StartupInfo.cb = if use_attr {
                    std::mem::size_of::<STARTUPINFOEXW>() as u32
                } else {
                    std::mem::size_of::<STARTUPINFOW>() as u32
                };
                if let Some(tok) = token {
                    let token_flags = if env.is_some() {
                        flags_to_use | CREATE_UNICODE_ENVIRONMENT
                    } else {
                        flags_to_use
                    };
                    let env_ptr =
                        env.map_or(std::ptr::null(), |e| e.as_ptr() as *const core::ffi::c_void);
                    CreateProcessWithTokenW(
                        tok,
                        CREATE_PROCESS_LOGON_FLAGS(0),
                        PCWSTR(exe_wide.as_ptr()),
                        Some(PWSTR(cmd_wide.as_mut_ptr())),
                        token_flags,
                        if env_ptr.is_null() {
                            None
                        } else {
                            Some(env_ptr)
                        },
                        work_pcwstr,
                        &si_copy.StartupInfo,
                        proc_info,
                    )
                } else {
                    CreateProcessW(
                        PCWSTR(exe_wide.as_ptr()),
                        Some(PWSTR(cmd_wide.as_mut_ptr())),
                        None,
                        None,
                        false,
                        flags_to_use,
                        None,
                        work_pcwstr,
                        &si_copy.StartupInfo,
                        proc_info,
                    )
                }
            };

        let res = invoke_spawn(with_attr, f, &mut pi);
        if res.is_err() && with_attr {
            log(&format!(
                "[warn] spawn with parent failed (err={:?}) - retrying as child",
                GetLastError()
            ));
            invoke_spawn(false, flags, &mut pi).map(|()| pi)
        } else {
            res.map(|()| pi)
        }
    }
}
