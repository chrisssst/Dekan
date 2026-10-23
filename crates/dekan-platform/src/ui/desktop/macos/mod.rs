pub mod clipboard;
pub mod dialog;
pub mod hotkey;
pub mod shell;

use std::process::{Command, Output};

const OSASCRIPT: &str = "/usr/bin/osascript";
const USER_CANCELED: &str = "(-128)";

pub(crate) fn run_script(lines: &[&str], args: &[&str]) -> std::io::Result<Output> {
    let mut command = Command::new(OSASCRIPT);
    command.args(["-e", "on run argv"]);
    for line in lines {
        command.args(["-e", line]);
    }
    command.args(["-e", "end run"]).args(args).output()
}

pub(crate) fn was_canceled(output: &Output) -> bool {
    String::from_utf8_lossy(&output.stderr)
        .trim()
        .ends_with(USER_CANCELED)
}
