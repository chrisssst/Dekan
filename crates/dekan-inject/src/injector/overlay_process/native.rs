use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::{OverlayProcess, PatcherSignal};
use crate::error::InjectError;

const STAR_GATE_FILE_ENV: &str = "BULLET_STAR_GATE_FILE";

pub(super) struct HookLaunch<'a> {
    pub dll_path: &'a Path,
    pub window_title: &'a str,
    pub image_name: &'a str,
}

pub(super) async fn spawn_native_host(
    host_exe: &Path,
    overlay_dir: &Path,
    pipe_name: &str,
    target_pid: Option<u32>,
    hook_target: Option<HookLaunch<'_>>,
) -> Result<OverlayProcess, InjectError> {
    let program_str = host_exe.display().to_string();
    if !host_exe.is_file() {
        return Err(InjectError::Process(format!(
            "Native patcher host not found at '{program_str}'"
        )));
    }

    let mut cmd = Command::new(host_exe);
    cmd.arg("--overlay-dir").arg(overlay_dir);
    cmd.arg("--pipe-name").arg(pipe_name);
    if let Some(pid) = target_pid {
        cmd.arg("--target-pid").arg(pid.to_string());
    }
    if let Some(target) = hook_target {
        cmd.arg("--dll-path").arg(target.dll_path);
        cmd.arg("--target-window-title").arg(target.window_title);
        cmd.arg("--expected-image-name").arg(target.image_name);
        cmd.arg("--shutdown-on-stdin-eof").arg("true");
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    if let Some(dir) = host_exe.parent() {
        cmd.current_dir(dir);
    }
    match dekan_platform::paths::state_dir() {
        Ok(dir) => {
            cmd.env(
                STAR_GATE_FILE_ENV,
                dir.join(dekan_platform::paths::STAR_GATE_FILE),
            );
        }
        Err(e) => {
            warn!(error = %e, "The native host gets no StarGate file path and uses its default")
        }
    }
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    info!(
        host = %program_str,
        overlay = %overlay_dir.display(),
        pipe = pipe_name,
        target_pid = ?target_pid,
        "Spawning Dekan Native Host process"
    );

    let mut child = cmd.spawn().map_err(|e| {
        InjectError::Process(format!("failed to spawn native host '{program_str}': {e}"))
    })?;

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::unbounded_channel();

    let tx_stdout = tx.clone();
    let stdout_reader = stdout.map(|out| {
        tokio::spawn(async move {
            let mut reader = BufReader::new(out);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim();
                debug!(target: "overlay::native-host", "{}", trimmed);
                if let Some(signal) = native_signal(trimmed) {
                    let _ = tx_stdout.send(signal); // ignore-ok: dispatch observed native host state
                }
                line.clear();
            }
        })
    });

    let tx_stderr = tx.clone();
    let stderr_reader = stderr.map(|err| {
        tokio::spawn(async move {
            let mut reader = BufReader::new(err);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim();
                warn!(target: "overlay::native-stderr", "{}", trimmed);
                if let Some(failure) = parse_native_failure(trimmed) {
                    error!(
                        target: "overlay::native-structured-error",
                        category = ?failure.category,
                        win32_code = ?failure.win32_code.map(|c| format!("0x{c:08x}")),
                        message = %failure.raw_message,
                        "Native host reported failure on stderr"
                    );
                    let _ = tx_stderr.send(PatcherSignal::Failed); // ignore-ok: dispatch failed signal to receiver
                }
                line.clear();
            }
        })
    });

    Ok(OverlayProcess {
        child: Some(child),
        _stdin: stdin,
        signals: rx,
        _stdout_reader: stdout_reader,
        _stderr_reader: stderr_reader,
        hooked: false,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeFailureCategory {
    Startup,
    HookRegistration,
    DllInitialization,
    Ipc,
    Overlay,
    General,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStructuredFailure {
    pub category: NativeFailureCategory,
    pub raw_message: String,
    pub win32_code: Option<u32>,
}

pub fn parse_native_failure(line: &str) -> Option<NativeStructuredFailure> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();

    let is_explicit_warning = (lower.contains("warn") || lower.contains("warning"))
        && !lower.contains("error:")
        && !lower.contains("fatal")
        && !lower.contains("panic");
    if is_explicit_warning {
        return None;
    }

    let has_success_qualifier = lower.contains("without error")
        || lower.contains("no error")
        || lower.contains("zero error")
        || lower.contains("0 error")
        || lower.contains("invalidated successfully")
        || lower.contains("interception confirmed")
        || lower.contains("acknowledged handshake");
    if has_success_qualifier {
        return None;
    }

    let is_failure = lower.contains("error:")
        || lower.contains("error(")
        || lower.contains("error ")
        || lower.starts_with("error")
        || lower.contains("[error]")
        || lower.contains("level = error")
        || lower.contains("fatal")
        || lower.contains("panic")
        || lower.contains("timed out waiting for dll")
        || lower.contains("peer mismatch")
        || lower.contains("handshake peer mismatch")
        || lower.contains("host_session_failed")
        || lower.contains("host_invalid_argument")
        || lower.contains("unsupported handshake schema")
        || lower.contains("setwindowshookex")
        || lower.contains("postthreadmessage")
        || lower.contains("rejected by kernel")
        || lower.contains("anti-cheat")
        || lower.contains("failed to connect")
        || lower.contains("failed to spawn")
        || lower.contains("failed to start")
        || lower.contains("failed to initialize")
        || lower.contains("connection refused");

    if !is_failure {
        return None;
    }

    let category = if lower.contains("setwindowshookex")
        || lower.contains("postthreadmessage")
        || lower.contains("loadlibrary")
        || lower.contains("hook procedure")
        || lower.contains("hookproc")
    {
        NativeFailureCategory::HookRegistration
    } else if lower.contains("named pipe")
        || lower.contains("handshake")
        || lower.contains("connectiontimeout")
        || lower.contains("timed out waiting for dll")
        || lower.contains("pipe_name")
        || lower.contains("peer mismatch")
        || lower.contains("connection refused")
    {
        NativeFailureCategory::Ipc
    } else if lower.contains("host_invalid")
        || lower.contains("option requires")
        || lower.contains("overlay directory does not exist")
        || lower.contains("hook dll does not exist")
        || lower.contains("target pid")
        || lower.contains("failed to spawn")
    {
        NativeFailureCategory::Startup
    } else if lower.contains("overlayconfigured")
        || lower.contains("overlay open failed")
        || lower.contains("refusing overlay")
        || lower.contains("virtualoverlayengine")
    {
        NativeFailureCategory::Overlay
    } else if lower.contains("dll") || lower.contains("ipc runtime") {
        NativeFailureCategory::DllInitialization
    } else {
        NativeFailureCategory::General
    };

    let win32_code = extract_win32_hex_code(trimmed);

    Some(NativeStructuredFailure {
        category,
        raw_message: trimmed.to_string(),
        win32_code,
    })
}

fn extract_win32_hex_code(line: &str) -> Option<u32> {
    if let Some(idx) = line.find("(0x") {
        let hex_str = &line[idx + 3..];
        let hex_digits: String = hex_str
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if !hex_digits.is_empty() {
            return u32::from_str_radix(&hex_digits, 16).ok();
        }
    }
    if let Some(idx) = line.find("0x") {
        let hex_str = &line[idx + 2..];
        let hex_digits: String = hex_str
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if !hex_digits.is_empty() && hex_digits.len() <= 8 {
            return u32::from_str_radix(&hex_digits, 16).ok();
        }
    }
    None
}

pub fn native_signal(line: &str) -> Option<PatcherSignal> {
    if line.contains("Waiting for DLL to connect to Named Pipe")
        || line.contains("Native target watcher armed")
    {
        Some(PatcherSignal::Armed)
    } else if line.contains("Native file-open interception confirmed by DLL") {
        Some(PatcherSignal::Hooked)
    } else if let Some(failure) = parse_native_failure(line) {
        error!(
            target: "overlay::native-structured-error",
            category = ?failure.category,
            win32_code = ?failure.win32_code.map(|c| format!("0x{c:08x}")),
            message = %failure.raw_message,
            "Native host failure detected in stdout"
        );
        Some(PatcherSignal::Failed)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
