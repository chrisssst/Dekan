use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tracing::debug;

use crate::error::PlatformError;

pub use super::super::ElevatedRun;

const OSASCRIPT: &str = "/usr/bin/osascript";
const ELEVATED_WAIT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(100);
const USER_CANCELED: &str = "(-128)";

pub fn run_elevated(exe: &Path, parameters: &str) -> Result<ElevatedRun, PlatformError> {
    let io = |context: &str| {
        let context = context.to_owned();
        move |source| PlatformError::Io { context, source }
    };
    let mut child = Command::new(OSASCRIPT)
        .args([
            "-e",
            "on run argv",
            "-e",
            "set command to \"\"",
            "-e",
            "repeat with argument in argv",
            "-e",
            "set command to command & quoted form of (argument as text) & \" \"",
            "-e",
            "end repeat",
            "-e",
            "do shell script command with administrator privileges",
            "-e",
            "end run",
        ])
        .arg(exe)
        .args(split_parameters(parameters))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io("starting the administrator prompt"))?;

    let deadline = Instant::now() + ELEVATED_WAIT;
    let status = loop {
        match child
            .try_wait()
            .map_err(io("waiting for the elevated process"))?
        {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(POLL),
            None => {
                let _ = child.kill(); // ignore-ok: the timeout below is the outcome reported
                return Err(PlatformError::Io {
                    context: format!(
                        "the elevated process did not finish within {} s",
                        ELEVATED_WAIT.as_secs()
                    ),
                    source: std::io::ErrorKind::TimedOut.into(),
                });
            }
        }
    };
    if status.success() {
        return Ok(ElevatedRun::Finished(0));
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr); // ignore-ok: an unreadable message falls through to the parse error below
    }
    parse_failure(&stderr)
}

fn split_parameters(parameters: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in parameters.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        arguments.push(current);
    }
    arguments
}

fn parse_failure(stderr: &str) -> Result<ElevatedRun, PlatformError> {
    let message = stderr.trim();
    if message.ends_with(USER_CANCELED) {
        return Ok(ElevatedRun::Declined);
    }
    let code = message
        .rsplit_once('(')
        .and_then(|(_, tail)| tail.strip_suffix(')'))
        .and_then(|code| code.parse::<u32>().ok());
    match code {
        Some(code) => Ok(ElevatedRun::Finished(code)),
        None => {
            debug!(
                message,
                "The administrator prompt failed without an exit code"
            );
            Err(PlatformError::Window(format!(
                "the administrator prompt failed: {message}"
            )))
        }
    }
}

pub fn is_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_become_separate_arguments_the_shell_never_reads() {
        assert_eq!(
            split_parameters(r#"--install-injector "1.29.3" "/Applications/My $(x) App/tools""#),
            vec![
                "--install-injector".to_owned(),
                "1.29.3".to_owned(),
                "/Applications/My $(x) App/tools".to_owned(),
            ]
        );
        assert_eq!(
            split_parameters(r#"  "" a "#),
            vec![String::new(), "a".to_owned()]
        );
    }

    #[test]
    fn a_canceled_prompt_is_declined_and_an_exit_code_is_kept() {
        assert_eq!(
            parse_failure("0:120: execution error: User canceled. (-128)").ok(),
            Some(ElevatedRun::Declined)
        );
        assert_eq!(
            parse_failure("0:120: execution error: copy failed (3)").ok(),
            Some(ElevatedRun::Finished(3))
        );
        assert!(parse_failure("garbled").is_err());
    }
}
