use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use tracing::warn;

use super::{run_script, was_canceled};
use crate::error::PlatformError;

pub use super::super::is_web_page;

const OPEN: &str = "/usr/bin/open";

pub fn open_folder(dir: &Path) -> Result<(), PlatformError> {
    if !dir.is_dir() {
        return Err(PlatformError::Path(format!(
            "'{}' is not an existing folder",
            dir.display()
        )));
    }
    open(dir.as_os_str(), &dir.display().to_string())
}

pub fn open_web_page(url: &str) -> Result<(), PlatformError> {
    if !is_web_page(url) {
        return Err(PlatformError::Path(format!("'{url}' is not an https page")));
    }
    open(OsStr::new(url), url)
}

fn open(target: &OsStr, shown: &str) -> Result<(), PlatformError> {
    let status = Command::new(OPEN)
        .arg(target)
        .status()
        .map_err(|source| PlatformError::Io {
            context: format!("starting open for '{shown}'"),
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::Window(format!(
            "open could not show '{shown}' ({status})"
        )))
    }
}

pub fn message_box(title: &str, text: &str) {
    show(title, text, "note");
}

pub fn message_box_warning(title: &str, text: &str) {
    show(title, text, "caution");
}

fn show(title: &str, text: &str, icon: &str) {
    let line = format!(
        "display dialog (item 1 of argv) with title (item 2 of argv) buttons {{\"OK\"}} default button 1 with icon {icon}"
    );
    match run_script(&[&line], &[text, title]) {
        Ok(output) if output.status.success() || was_canceled(&output) => {}
        Ok(output) => warn!(
            title,
            error = %String::from_utf8_lossy(&output.stderr).trim(),
            "macOS could not show a message box"
        ),
        Err(e) => warn!(title, error = %e, "macOS could not show a message box"),
    }
}

#[must_use]
pub fn message_box_question(title: &str, text: &str) -> bool {
    match run_script(
        &["display dialog (item 1 of argv) with title (item 2 of argv) with icon note"],
        &[text, title],
    ) {
        Ok(output) if output.status.success() => true,
        Ok(output) if was_canceled(&output) => false,
        Ok(output) => {
            warn!(
                title,
                error = %String::from_utf8_lossy(&output.stderr).trim(),
                "macOS could not show a question"
            );
            false
        }
        Err(e) => {
            warn!(title, error = %e, "macOS could not show a question");
            false
        }
    }
}

pub fn dismiss_question(_title: &str) -> bool {
    false
}

pub(crate) fn notify(title: &str, body: &str) -> bool {
    match run_script(
        &["display notification (item 1 of argv) with title (item 2 of argv)"],
        &[body, title],
    ) {
        Ok(output) => output.status.success(),
        Err(e) => {
            warn!(title, error = %e, "macOS could not show a notification");
            false
        }
    }
}
