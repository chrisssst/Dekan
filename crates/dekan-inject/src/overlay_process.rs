use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::error::InjectError;
use crate::ltk_host::{self, HostEvent, HostLogLevel, HostState, StderrLevel};

fn is_dll_failure(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("failed") || lower.contains("error") || lower.contains("unable to")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayLine {
    pub is_stderr: bool,

    pub text: String,
}

pub struct OverlayProcess {
    child: Option<Child>,

    _stdin: Option<ChildStdin>,
    lines: mpsc::UnboundedReceiver<OverlayLine>,
    _stdout_reader: Option<tokio::task::JoinHandle<()>>,
    _stderr_reader: Option<tokio::task::JoinHandle<()>>,
}

impl OverlayProcess {
    pub fn spawn(program: &Path, args: &[String]) -> Result<Self, InjectError> {
        let program_str = program.display().to_string();

        if !program.is_file() {
            return Err(InjectError::Process(format!(
                "overlay program not found at '{program_str}'"
            )));
        }

        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);

        info!(program = %program_str, args = ?args, "Spawning overlay process");

        let mut child = cmd
            .spawn()
            .map_err(|e| InjectError::Process(format!("failed to spawn '{program_str}': {e}")))?;

        let child_pid = child.id();
        info!(
            program = %program_str,
            pid = ?child_pid,
            args = ?args,
            "Overlay process spawned successfully"
        );

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let (tx, rx) = mpsc::unbounded_channel();

        let stdout_reader = stdout.map(|out| {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(out).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if line.starts_with("Status:") {
                        info!(target: "overlay::status", line = %line, "Patcher status update");
                    } else if is_dll_failure(&line) {

                        warn!(target: "overlay::dll", line = %line, "Patcher DLL reported a failure");
                    } else {
                        debug!(target: "overlay::stdout", "{}", line);
                    }
                    if tx
                        .send(OverlayLine {
                            is_stderr: false,
                            text: line,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
        });

        let stderr_reader = stderr.map(|err| {
            tokio::spawn(async move {
                let mut reader = BufReader::new(err).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    warn!(target: "overlay::stderr", "{}", line);
                    if tx
                        .send(OverlayLine {
                            is_stderr: true,
                            text: line,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
        });

        Ok(Self {
            child: Some(child),
            _stdin: stdin,
            lines: rx,
            _stdout_reader: stdout_reader,
            _stderr_reader: stderr_reader,
        })
    }

    pub async fn spawn_ltk_host(
        host_exe: &Path,
        prefix: &Path,
        flags: u32,
        log_level: HostLogLevel,
    ) -> Result<Self, InjectError> {
        let program_str = host_exe.display().to_string();
        if !host_exe.is_file() {
            return Err(InjectError::Process(format!(
                "LTK patcher host not found at '{program_str}'"
            )));
        }

        let mut prefix_str = prefix.to_string_lossy().to_string();
        if !prefix_str.ends_with(['\\', '/']) {
            prefix_str.push('\\');
        }

        let mut cmd = Command::new(host_exe);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        if let Some(dir) = host_exe.parent() {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);

        info!(
            host = %program_str,
            prefix = %prefix_str,
            flags,
            elevated = dekan_platform::elevation::is_elevated(),
            "Spawning LTK patcher host"
        );

        let mut child = cmd.spawn().map_err(|e| {
            InjectError::Process(format!("failed to spawn LTK host '{program_str}': {e}"))
        })?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| InjectError::Process("LTK host stdin unavailable".into()))?;
        let commands = format!(
            "config loglevel {}\nconfig flags {}\nconfig prefix {}\nstart scan\n",
            log_level as u32, flags, prefix_str
        );
        stdin
            .write_all(commands.as_bytes())
            .await
            .map_err(|e| InjectError::Process(format!("failed to configure LTK host: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| InjectError::Process(format!("failed to flush LTK host config: {e}")))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, rx) = mpsc::unbounded_channel();

        let stdout_reader = stdout.map(|out| {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(out).lines();
                while let Ok(Some(raw)) = reader.next_line().await {
                    let Some(event) = ltk_host::parse_host_event(&raw) else {
                        debug!(target: "overlay::ltk", "{}", raw);
                        continue;
                    };

                    let sentinel = match &event {
                        HostEvent::Status { state, message } => match state {
                            HostState::Injecting => {
                                info!(target: "overlay::ltk", message = %message, "Patcher armed; scanning for the game");
                                Some(format!("Status: {}", crate::pipeline::PATCHER_ARMED_STATUS))
                            }
                            HostState::Injected | HostState::Waiting => {
                                info!(target: "overlay::ltk", message = %message, "DLL attached to the game; overlay active");
                                Some(format!("Status: {}", crate::pipeline::HOOK_CONFIRMED_STATUS))
                            }
                            HostState::Exited => {
                                info!(target: "overlay::ltk", message = %message, "Game exited; overlay session ending");
                                None
                            }
                            HostState::Failed => {
                                if message.contains("SetWindowsHookEx failed") {

                                    error!(
                                        target: "overlay::ltk",
                                        reason = %message,
                                        "Injection failed: the patcher DLL could not install its hook; the skin will not load"
                                    );
                                } else {
                                    error!(target: "overlay::ltk", reason = %message, "Injection failed; the skin will not load");
                                }
                                None
                            }
                        },
                        HostEvent::DllLog { level, message } => {
                            if ltk_host::is_end_of_life(message) {
                                error!(target: "overlay::dll", reason = %message, "Patcher DLL reached end of life; a refreshed DLL is required for this game patch");
                            } else if ltk_host::is_antihack_bypassed(message) {
                                info!(target: "overlay::ah", reason = %message, "Mod contains non-standard skin structure (c0000229); safely bypassed via OPT_OUT_AH_V1");
                            } else if ltk_host::is_dll_failure(level, message) {
                                warn!(target: "overlay::dll", level = %level, "{}", message);
                            } else {
                                debug!(target: "overlay::dll", level = %level, "{}", message);
                            }
                            None
                        }
                        HostEvent::Error { message } => {
                            warn!(target: "overlay::ltk", "{}", message);
                            None
                        }
                        HostEvent::Ok { message } => {
                            debug!(target: "overlay::ltk", "ok {}", message);
                            None
                        }
                    };
                    if let Some(text) = sentinel {
                        if tx.send(OverlayLine { is_stderr: false, text }).is_err() {
                            break;
                        }
                    }
                }
            })
        });

        let stderr_reader = stderr.map(|err| {
            tokio::spawn(async move {
                let mut reader = BufReader::new(err).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    match ltk_host::stderr_level(&line) {
                        StderrLevel::Error => error!(target: "overlay::ltk-stderr", "{}", line),
                        StderrLevel::Warn | StderrLevel::Unknown => {
                            warn!(target: "overlay::ltk-stderr", "{}", line)
                        }
                        StderrLevel::Info => info!(target: "overlay::ltk-stderr", "{}", line),
                        StderrLevel::Debug => debug!(target: "overlay::ltk-stderr", "{}", line),
                    }
                }
            })
        });

        Ok(Self {
            child: Some(child),
            _stdin: Some(stdin),
            lines: rx,
            _stdout_reader: stdout_reader,
            _stderr_reader: stderr_reader,
        })
    }

    pub async fn wait_for_line<F>(
        &mut self,
        predicate: F,
        timeout: Duration,
    ) -> Result<OverlayLine, InjectError>
    where
        F: Fn(&str) -> bool,
    {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(InjectError::HookUnconfirmed {
                    timeout_ms: timeout.as_millis() as u64,
                });
            }

            match tokio::time::timeout(remaining, self.lines.recv()).await {
                Ok(Some(line)) => {
                    if predicate(&line.text) {
                        return Ok(line);
                    }
                }
                Ok(None) => {
                    return Err(InjectError::Process(
                        "overlay process closed its output before signalling".into(),
                    ));
                }
                Err(_) => {
                    return Err(InjectError::HookUnconfirmed {
                        timeout_ms: timeout.as_millis() as u64,
                    });
                }
            }
        }
    }

    pub fn exited(&mut self) -> Option<i32> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            _ => None,
        }
    }

    pub async fn exited_within(&mut self, grace: Duration) -> Option<i32> {
        let child = self.child.as_mut()?;
        match tokio::time::timeout(grace, child.wait()).await {
            Ok(Ok(status)) => Some(status.code().unwrap_or(-1)),
            _ => None,
        }
    }

    pub async fn shutdown(mut self) {
        if let Some(h) = self._stdout_reader.take() {
            h.abort();
        }
        if let Some(h) = self._stderr_reader.take() {
            h.abort();
        }

        drop(self._stdin.take());

        if let Some(mut child) = self.child.take() {
            match tokio::time::timeout(Duration::from_millis(150), child.wait()).await {
                Ok(Ok(status)) => {
                    info!(exit_code = ?status.code(), "Overlay process exited cleanly on stdin close");
                }
                _ => match child.kill().await {
                    Ok(()) => {
                        let code = child.wait().await.ok().and_then(|s| s.code());
                        info!(exit_code = ?code, "Overlay process terminated");
                    }
                    Err(e) => warn!(error = %e, "Failed to terminate overlay process"),
                },
            }
        }
    }
}

impl Drop for OverlayProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            if let Err(e) = child.start_kill() {
                debug!(error = %e, "Overlay process already gone at drop");
            }
        }
        if let Some(h) = self._stdout_reader.take() {
            h.abort();
        }
        if let Some(h) = self._stderr_reader.take() {
            h.abort();
        }
    }
}

#[cfg(test)]
#[path = "overlay_process_tests.rs"]
mod tests;
