use std::path::{Path, PathBuf};

use crate::error::PlatformError;

const LABEL: &str = "dev.dekan.app";

pub fn is_enabled() -> Result<bool, PlatformError> {
    let exe = current_exe()?;
    match std::fs::read_to_string(agent_path()?) {
        Ok(plist) => Ok(plist.contains(&program_entry(&exe))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(PlatformError::Io {
            context: "reading the login item".into(),
            source,
        }),
    }
}

pub fn enable() -> Result<(), PlatformError> {
    let exe = current_exe()?;
    crate::fs::atomic_write(&agent_path()?, launch_agent(&exe).as_bytes(), true)
}

pub fn disable() -> Result<(), PlatformError> {
    match std::fs::remove_file(agent_path()?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(PlatformError::Io {
            context: "removing the login item".into(),
            source,
        }),
    }
}

pub fn toggle() -> Result<bool, PlatformError> {
    if is_enabled()? {
        disable()?;
        Ok(false)
    } else {
        enable()?;
        Ok(true)
    }
}

fn agent_path() -> Result<PathBuf, PlatformError> {
    let home = std::env::home_dir()
        .ok_or_else(|| PlatformError::Path("the user has no home folder".into()))?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

fn current_exe() -> Result<PathBuf, PlatformError> {
    std::env::current_exe().map_err(|e| PlatformError::Io {
        context: "failed to resolve the running executable".into(),
        source: e,
    })
}

fn program_entry(exe: &Path) -> String {
    let escaped = exe
        .display()
        .to_string()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<string>{escaped}</string>")
}

fn launch_agent(exe: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n\
<key>Label</key>\n<string>{LABEL}</string>\n\
<key>ProgramArguments</key>\n<array>\n{}\n</array>\n\
<key>RunAtLoad</key>\n<true/>\n\
<key>ProcessType</key>\n<string>Interactive</string>\n\
</dict>\n</plist>\n",
        program_entry(exe)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_runs_the_escaped_executable_at_login() {
        let plist = launch_agent(Path::new("/Applications/A&B.app/Contents/MacOS/dekan"));
        assert!(plist.contains("<string>/Applications/A&amp;B.app/Contents/MacOS/dekan</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n<true/>"));
        assert!(plist.contains(&format!("<string>{LABEL}</string>")));
    }
}
