use std::path::Path;
use std::time::Duration;

use tokio::process::{Child, ChildStdin};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::error::InjectError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatcherSignal {
    Armed,

    Hooked,

    Failed,
}

pub struct OverlayProcess {
    pub(crate) child: Option<Child>,

    pub(crate) _stdin: Option<ChildStdin>,
    pub(crate) signals: mpsc::UnboundedReceiver<PatcherSignal>,
    pub(crate) _stdout_reader: Option<tokio::task::JoinHandle<()>>,
    pub(crate) _stderr_reader: Option<tokio::task::JoinHandle<()>>,
    pub(crate) hooked: bool,
}

const READER_DRAIN: Duration = Duration::from_millis(500);

impl OverlayProcess {
    pub async fn spawn_native_host(
        host_exe: &Path,
        overlay_dir: &Path,
        pipe_name: &str,
        target_pid: Option<u32>,
    ) -> Result<Self, InjectError> {
        native::spawn_native_host(host_exe, overlay_dir, pipe_name, target_pid, None).await
    }

    pub async fn spawn_native_game_host(
        host_exe: &Path,
        dll_path: &Path,
        overlay_dir: &Path,
        pipe_name: &str,
        window_title: &str,
        image_name: &str,
    ) -> Result<Self, InjectError> {
        native::spawn_native_host(
            host_exe,
            overlay_dir,
            pipe_name,
            None,
            Some(native::HookLaunch {
                dll_path,
                window_title,
                image_name,
            }),
        )
        .await
    }

    pub async fn wait_for(
        &mut self,
        wanted: PatcherSignal,
        timeout: Duration,
    ) -> Result<(), InjectError> {
        let unconfirmed = || InjectError::HookUnconfirmed {
            timeout_ms: timeout.as_millis() as u64,
        };
        if self.hooked {
            return Ok(());
        }
        let wait = async {
            loop {
                match self.signals.recv().await {
                    Some(PatcherSignal::Hooked) => {
                        self.hooked = true;
                        return Ok(());
                    }
                    Some(PatcherSignal::Failed) => return Err(InjectError::HookFailed),
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

    pub fn ready_for_next_game(&mut self) -> Result<(), i32> {
        if let Some(code) = self.exited() {
            return Err(code);
        }
        self.hooked = false;
        while self.signals.try_recv().is_ok() {}
        Ok(())
    }

    pub fn exited(&mut self) -> Option<i32> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            Ok(None) => None,
            Err(e) => {
                warn!(error = %e, "The patcher's exit status could not be read");
                None
            }
        }
    }

    pub async fn exited_within(&mut self, grace: Duration) -> Option<i32> {
        let child = self.child.as_mut()?;
        match tokio::time::timeout(grace, child.wait()).await {
            Ok(Ok(status)) => Some(status.code().unwrap_or(-1)),
            Ok(Err(e)) => {
                warn!(error = %e, "The patcher's exit status could not be read");
                None
            }
            Err(_) => None,
        }
    }

    pub async fn shutdown(mut self) {
        drop(self._stdin.take());

        if let Some(mut child) = self.child.take() {
            match tokio::time::timeout(Duration::from_millis(150), child.wait()).await {
                Ok(Ok(status)) => {
                    info!(exit_code = ?status.code(), "Overlay process exited cleanly on stdin close");
                }
                _ => match child.kill().await {
                    Ok(()) => match child.wait().await {
                        Ok(status) => {
                            info!(exit_code = ?status.code(), "Overlay process terminated");
                        }
                        Err(e) => {
                            warn!(error = %e, "Overlay process killed; its exit status could not be read");
                        }
                    },
                    Err(e) => warn!(error = %e, "Failed to terminate overlay process"),
                },
            }
        }

        for (stream, reader) in [
            ("stdout", self._stdout_reader.take()),
            ("stderr", self._stderr_reader.take()),
        ] {
            let Some(reader) = reader else {
                continue;
            };
            let abort = reader.abort_handle();
            match tokio::time::timeout(READER_DRAIN, reader).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => warn!(stream, error = %e, "Patcher output reader failed"),
                Err(_) => {
                    abort.abort();
                    debug!(
                        stream,
                        "Patcher output still open after the patcher exited; reader stopped"
                    );
                }
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

pub(crate) mod native;

#[cfg(test)]
mod tests;
