use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use dekan_platform::fs::get_disk_free_space;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{debug, error, info, warn};

use crate::error::InjectError;

#[derive(Debug, Clone)]
pub struct SubprocessOutput {
    pub exit_code: i32,

    pub stdout_lines: Vec<String>,

    pub stderr_lines: Vec<String>,
}

pub fn ensure_sufficient_disk_space(
    target_path: &Path,
    min_required_bytes: u64,
) -> Result<u64, InjectError> {
    let free_bytes = get_disk_free_space(target_path)?;
    if free_bytes < min_required_bytes {
        return Err(InjectError::InsufficientDiskSpace {
            path: target_path.display().to_string(),
            required_bytes: min_required_bytes,
            available_bytes: free_bytes,
        });
    }
    Ok(free_bytes)
}

pub async fn run_subprocess(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    timeout_duration: Duration,
) -> Result<SubprocessOutput, InjectError> {
    let program_str = program.display().to_string();

    let mut cmd = Command::new(program);
    cmd.args(args);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    cmd.kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    info!(program = %program_str, args = ?args, "Spawning subprocess");

    let mut child = cmd.spawn().map_err(|e| {
        error!(error = %e, program = %program_str, "Failed to spawn subprocess");
        InjectError::Process(format!("failed to spawn '{program_str}': {e}"))
    })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stdout_reader = tokio::spawn(async move {
        let mut lines = Vec::new();
        if let Some(out) = stdout {
            let mut reader = BufReader::new(out).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                debug!(target: "subprocess::stdout", "{}", line);
                if lines.len() < 100 {
                    lines.push(line);
                }
            }
        }
        lines
    });

    let stderr_reader = tokio::spawn(async move {
        let mut lines = Vec::new();
        if let Some(err) = stderr {
            let mut reader = BufReader::new(err).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                warn!(target: "subprocess::stderr", "{}", line);
                if lines.len() < 100 {
                    lines.push(line);
                }
            }
        }
        lines
    });

    let wait_result = tokio::time::timeout(timeout_duration, child.wait()).await;

    let exit_status = match wait_result {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            stdout_reader.abort();
            stderr_reader.abort();
            return Err(InjectError::Process(format!(
                "subprocess wait error for '{program_str}': {e}"
            )));
        }
        Err(_) => {
            warn!(
                program = %program_str,
                timeout_secs = timeout_duration.as_secs(),
                "Subprocess execution timed out; terminating child"
            );
            let _ = child.kill().await; // ignore-ok: killing after a timeout; a failure means the process already died
            stdout_reader.abort();
            stderr_reader.abort();
            return Err(InjectError::SubprocessTimeout {
                command: program_str,
                timeout_secs: timeout_duration.as_secs(),
            });
        }
    };

    let stdout_lines = stdout_reader.await.unwrap_or_default();
    let stderr_lines = stderr_reader.await.unwrap_or_default();

    let exit_code = exit_status.code().unwrap_or(-1);

    if !exit_status.success() {
        let error_summary = if !stderr_lines.is_empty() {
            stderr_lines.join(" | ")
        } else if !stdout_lines.is_empty() {
            stdout_lines.join(" | ")
        } else {
            "no output generated".to_string()
        };

        error!(
            program = %program_str,
            exit_code = exit_code,
            details = %error_summary,
            "Subprocess failed with non-zero exit code"
        );

        return Err(InjectError::SubprocessFailed {
            exit_code,
            details: format!("{program_str}: {error_summary}"),
        });
    }

    info!(
        program = %program_str,
        exit_code = exit_code,
        "Subprocess completed successfully"
    );

    Ok(SubprocessOutput {
        exit_code,
        stdout_lines,
        stderr_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_disk_space_precheck() {
        let temp_dir = std::env::temp_dir();

        let res = ensure_sufficient_disk_space(&temp_dir, 1);
        assert!(res.is_ok());

        let absurd = 1024 * 1024 * 1024 * 1024 * 1024u64;
        let err = ensure_sufficient_disk_space(&temp_dir, absurd);
        assert!(matches!(
            err,
            Err(InjectError::InsufficientDiskSpace { .. })
        ));
    }

    #[tokio::test]
    async fn test_run_subprocess_success() {
        let cmd_exe = Path::new("cmd.exe");
        let args = vec!["/C".into(), "echo dekan_test_output".into()];

        let output = run_subprocess(cmd_exe, &args, None, Duration::from_secs(5))
            .await
            .expect("run echo");

        assert_eq!(output.exit_code, 0);
        assert!(
            output
                .stdout_lines
                .iter()
                .any(|l| l.contains("dekan_test_output"))
        );
    }

    #[tokio::test]
    async fn test_run_subprocess_failure_details() {
        let cmd_exe = Path::new("cmd.exe");
        let args = vec!["/C".into(), "exit 42".into()];

        let err = run_subprocess(cmd_exe, &args, None, Duration::from_secs(5))
            .await
            .unwrap_err();

        match err {
            InjectError::SubprocessFailed { exit_code, .. } => assert_eq!(exit_code, 42),
            other => panic!("expected SubprocessFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_subprocess_timeout() {
        let cmd_exe = Path::new("cmd.exe");

        let args = vec!["/C".into(), "ping 127.0.0.1 -n 3 > nul".into()];

        let err = run_subprocess(cmd_exe, &args, None, Duration::from_millis(100))
            .await
            .unwrap_err();

        assert!(matches!(err, InjectError::SubprocessTimeout { .. }));
    }
}
