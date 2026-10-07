use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::error::InjectError;
use crate::ltk_host::{self, HostEvent, HostLogLevel, HostState, StderrLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatcherSignal {
    Armed,

    Hooked,
}

pub struct OverlayProcess {
    child: Option<Child>,

    _stdin: Option<ChildStdin>,
    signals: mpsc::UnboundedReceiver<PatcherSignal>,
    _stdout_reader: Option<tokio::task::JoinHandle<()>>,
    _stderr_reader: Option<tokio::task::JoinHandle<()>>,
}

impl OverlayProcess {
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

        let stdout_reader = stdout.map(|out| tokio::spawn(read_host_stdout(out, tx)));

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
            signals: rx,
            _stdout_reader: stdout_reader,
            _stderr_reader: stderr_reader,
        })
    }

    pub async fn wait_for(
        &mut self,
        wanted: PatcherSignal,
        timeout: Duration,
    ) -> Result<(), InjectError> {
        let unconfirmed = || InjectError::HookUnconfirmed {
            timeout_ms: timeout.as_millis() as u64,
        };
        let wait = async {
            loop {
                match self.signals.recv().await {
                    Some(signal) if signal == wanted => return Ok(()),
                    Some(_) => {}
                    None => {
                        return Err(InjectError::Process(
                            "overlay process closed its output before signalling".into(),
                        ));
                    }
                }
            }
        };
        tokio::time::timeout(timeout, wait)
            .await
            .unwrap_or_else(|_| Err(unconfirmed()))
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

async fn read_host_stdout<R: AsyncRead + Unpin>(out: R, tx: mpsc::UnboundedSender<PatcherSignal>) {
    let mut reader = BufReader::new(out).lines();
    while let Ok(Some(raw)) = reader.next_line().await {
        let Some(event) = ltk_host::parse_host_event(&raw) else {
            debug!(target: "overlay::ltk", "{}", raw);
            continue;
        };

        let signal = match &event {
            HostEvent::Status { state, message } => match state {
                HostState::Injecting => {
                    info!(target: "overlay::ltk", message = %message, "Patcher armed; scanning for the game");
                    Some(PatcherSignal::Armed)
                }
                HostState::Injected | HostState::Waiting => {
                    info!(target: "overlay::ltk", message = %message, "DLL attached to the game; overlay active");
                    Some(PatcherSignal::Hooked)
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
                } else if ltk_host::is_expected_status(message) {
                    info!(target: "overlay::dll", reason = %message, "The patcher reported status c0000229, expected with the default hook flags");
                } else if let Some(wad) = ltk_host::redirected_wad(message) {
                    info!(target: "overlay::dll", wad = %wad, "The game opened this archive from the overlay");
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
        if let Some(signal) = signal {
            if tx.send(signal).is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
#[path = "overlay_process_tests.rs"]
mod tests;
