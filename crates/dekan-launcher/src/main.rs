mod cli;
mod env;
mod inject;
mod process;

use std::io::Write;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED, GetExitCodeProcess, OpenProcess,
    PROCESS_CREATE_PROCESS, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, ResumeThread, TerminateProcess, WaitForSingleObject,
};

use crate::cli::log;
use crate::process::ProcThreadAttributeListGuard;

fn main() {
    let args = cli::parse_args();
    cli::init_logger(args.log_dir.as_deref());
    log("========== dekan_launcher ==========");

    if args.as_user_child {
        unsafe {
            std::env::remove_var("__COMPAT_LAYER");
        }
    }

    let exe_path = args.exe_path.clone().or_else(inject::resolve_default_exe);
    let exe_path = match exe_path {
        Some(p) if p.is_file() => p,
        _ => {
            std::process::exit(cli::fail(
                args.quiet,
                cli::EXIT_EXE_MISSING,
                "ltk-manager.exe not found",
            ));
        }
    };

    let dll_path = match inject::resolve_dll_path() {
        Some(d) => d,
        None => {
            std::process::exit(cli::fail(
                args.quiet,
                cli::EXIT_DLL_MISSING,
                "dekan_bridge.dll must be in the same folder as dekan_launcher.exe.",
            ));
        }
    };

    let before = process::manager_pids();
    if !before.is_empty() {
        log(&format!(
            "[err] Mod Manager is already running (pid={})",
            before[0]
        ));
        std::process::exit(cli::fail(
            args.quiet,
            cli::EXIT_ALREADY_RUNNING,
            "Mod Manager is already open. Close it, then start it through this launcher.",
        ));
    }

    let cdp_port = env::pick_free_port();
    let vars = env::setup_environment(cdp_port, args.no_hide, args.log_dir.as_deref());
    log(&format!("[cdp] remote debugging on 127.0.0.1:{cdp_port}"));

    let work_dir = exe_path.parent();
    let flags = PROCESS_CREATION_FLAGS(CREATE_SUSPENDED.0 | CREATE_NEW_PROCESS_GROUP.0);

    let is_elevated = process::is_elevated();
    if !process::enable_privilege("SeDebugPrivilege") && is_elevated {
        log("[warn] SeDebugPrivilege could not be turned on");
    }

    let shell_pid = process::shell_pid();
    let shell_proc = shell_pid.and_then(|spid| unsafe {
        OpenProcess(
            PROCESS_CREATE_PROCESS | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            spid,
        )
        .ok()
    });

    let attr_guard = shell_proc.and_then(ProcThreadAttributeListGuard::new_parent);

    if attr_guard.is_some() {
        if let Some(spid) = shell_pid {
            log(&format!("[launch] explorer.exe (pid={spid}) as the parent"));
        }
    } else {
        log(&format!(
            "[warn] no explorer.exe (pid={:?}) to parent Mod Manager to",
            shell_pid
        ));
    }

    let mut cmd_line = format!("\"{}\" --shell-relaunched", exe_path.display());
    let mut started_pi: Option<PROCESS_INFORMATION> = None;
    let mut restart_expected = false;
    let mut helper_pid = 0u32;

    if is_elevated {
        let shell_token = shell_pid.and_then(process::shell_user_token);
        if let Some(token) = shell_token {
            let env_block = env::environment_for_token(token, &vars);
            let attr_ptr = attr_guard.as_ref().map(|g| g.as_ptr());

            if attr_ptr.is_some() {
                match process::spawn_with_user_token(
                    token,
                    &env_block,
                    &exe_path,
                    &cmd_line,
                    work_dir,
                    attr_ptr,
                    flags,
                    args.no_hide,
                ) {
                    Ok(pi) => {
                        started_pi = Some(pi);
                        log(
                            "[launch] started unelevated as the desktop user, shell parent in one call",
                        );
                    }
                    Err(e) => {
                        log(&format!("[warn] CreateProcessAsUser err={e}"));
                    }
                }
            }

            if started_pi.is_none() && attr_guard.is_some() {
                if let Some(hpid) = process::spawn_as_user_child(
                    token,
                    &env_block,
                    &exe_path,
                    args.no_hide,
                    args.log_dir.as_deref(),
                ) {
                    helper_pid = hpid;
                    log(
                        "[launch] started unelevated as the desktop user, shell parent by the helper",
                    );
                }
            }

            if started_pi.is_none() && helper_pid == 0 {
                match process::spawn_process(
                    Some(token),
                    Some(&env_block),
                    &exe_path,
                    &cmd_line,
                    work_dir,
                    attr_ptr,
                    flags,
                    args.no_hide,
                ) {
                    Ok(pi) => {
                        started_pi = Some(pi);
                        log("[launch] started unelevated as the desktop user");
                    }
                    Err(e) => {
                        log(&format!("[warn] CreateProcessWithTokenW err={e}"));
                    }
                }
            }
            let _ = unsafe { CloseHandle(token) }; // ignore-ok: shell token cleanup
        }

        if started_pi.is_none() && helper_pid == 0 {
            log("[warn] starting Mod Manager elevated: it restarts itself through the shell");
            cmd_line = format!("\"{}\"", exe_path.display());
            restart_expected = true;
        }
    }

    if helper_pid != 0 {
        drop(attr_guard);
        log(&format!("[launch] helper reported pid={helper_pid}"));
        println!("MANAGER_PID={helper_pid}");
        let _ = std::io::stdout().flush(); // ignore-ok: flush stdout
        log("========== launcher exiting ==========");
        std::process::exit(cli::EXIT_OK);
    }

    let pi = match started_pi {
        Some(pi) => pi,
        None => {
            let attr_ptr = attr_guard.as_ref().map(|g| g.as_ptr());
            match process::spawn_process(
                None,
                None,
                &exe_path,
                &cmd_line,
                work_dir,
                attr_ptr,
                flags,
                args.no_hide,
            ) {
                Ok(pi) => pi,
                Err(e) => {
                    log(&format!("[err] CreateProcessW err={e}"));
                    std::process::exit(cli::EXIT_SPAWN_FAILED);
                }
            }
        }
    };
    drop(attr_guard);

    let parent_pid = process::parent_pid_of(pi.dwProcessId).unwrap_or(0);
    let parent_desc = if shell_pid == Some(parent_pid) {
        "explorer.exe"
    } else {
        "not explorer.exe"
    };
    log(&format!(
        "[launch] spawned pid = {} (suspended), parent pid = {} ({})",
        pi.dwProcessId, parent_pid, parent_desc
    ));

    if !inject::inject_dll(pi.dwProcessId, &dll_path) {
        log("[err] injection failed - terminating ltk-manager.exe");
        unsafe {
            let _ = TerminateProcess(pi.hProcess, 1); // ignore-ok: terminate failed injection process
            let _ = CloseHandle(pi.hThread); // ignore-ok: cleanup thread
            let _ = CloseHandle(pi.hProcess); // ignore-ok: cleanup process
        }
        std::process::exit(cli::EXIT_INJECT_FAILED);
    }

    unsafe {
        ResumeThread(pi.hThread);
    }

    let mut live_pid = 0u32;
    let mut spawned_exited = false;
    let settle = if restart_expected { 3000 } else { 800 };
    let start = Instant::now();

    while start.elapsed() < Duration::from_millis(6000) {
        unsafe {
            spawned_exited = WaitForSingleObject(pi.hProcess, 0) == WAIT_OBJECT_0;
        }
        if let Some(fresh) = process::find_new_manager_pid(&before, pi.dwProcessId) {
            live_pid = fresh;
            break;
        }
        if !spawned_exited && start.elapsed() >= Duration::from_millis(settle) {
            live_pid = pi.dwProcessId;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    if live_pid == 0 && !spawned_exited {
        live_pid = pi.dwProcessId;
    }

    if spawned_exited || live_pid != pi.dwProcessId {
        let mut proc_exit = 0u32;
        unsafe {
            let _ = GetExitCodeProcess(pi.hProcess, &mut proc_exit); // ignore-ok: exit code read
        }
        log(&format!(
            "[launch] spawned pid={} handed over (exit code {})",
            pi.dwProcessId, proc_exit
        ));
    }

    unsafe {
        let _ = CloseHandle(pi.hThread); // ignore-ok: cleanup thread
        let _ = CloseHandle(pi.hProcess); // ignore-ok: cleanup process
    }

    if live_pid == 0 {
        log("[err] the spawned Mod Manager exited and no new instance came up");
        std::process::exit(cli::EXIT_NO_LIVE_INSTANCE);
    }

    if live_pid != pi.dwProcessId {
        log(&format!(
            "[re-inject] injecting into live ltk-manager.exe pid={live_pid}"
        ));
        if !inject::inject_dll(live_pid, &dll_path) {
            std::process::exit(cli::EXIT_INJECT_FAILED);
        }
    } else {
        log(&format!("[launch] pid={live_pid} is alive and running!"));
    }

    if !args.as_user_child {
        println!("MANAGER_PID={live_pid}");
        let _ = std::io::stdout().flush(); // ignore-ok: flush stdout
    }

    if let Some(ref pid_file) = args.pid_file {
        if let Err(e) = std::fs::write(pid_file, format!("{live_pid}")) {
            log(&format!(
                "[warn] could not write {} (err={e})",
                pid_file.display()
            ));
        }
    }

    log("========== launcher exiting ==========");
    std::process::exit(cli::EXIT_OK);
}
