use std::io::Write;
use std::process::{Command, Stdio};

use crate::error::PlatformError;

const PBCOPY: &str = "/usr/bin/pbcopy";
const PBPASTE: &str = "/usr/bin/pbpaste";

fn io(context: &'static str) -> impl FnOnce(std::io::Error) -> PlatformError {
    move |source| PlatformError::Io {
        context: context.into(),
        source,
    }
}

pub fn set_text(text: &str) -> Result<(), PlatformError> {
    let mut child = Command::new(PBCOPY)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(io("starting pbcopy"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(io("writing to the clipboard"))?;
    }
    let status = child.wait().map_err(io("waiting for pbcopy"))?;
    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::Window(format!("pbcopy failed ({status})")))
    }
}

pub fn get_text() -> Result<Option<String>, PlatformError> {
    let output = Command::new(PBPASTE)
        .output()
        .map_err(io("starting pbpaste"))?;
    if !output.status.success() {
        return Err(PlatformError::Window(format!(
            "pbpaste failed ({})",
            output.status
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok((!text.is_empty()).then_some(text))
}
