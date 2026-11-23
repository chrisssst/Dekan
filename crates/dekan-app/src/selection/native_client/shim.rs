use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

const IFEO: &str =
    r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\LeagueClientUx.exe";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Registration {
    Missing,
    Ours,
    Other,
}

pub(super) fn registration(launcher: &Path) -> Registration {
    match dekan_platform::shell::read_hklm_string(IFEO, "Debugger") {
        None => Registration::Missing,
        Some(value) if owns(&value, launcher) => Registration::Ours,
        Some(_) => Registration::Other,
    }
}

pub(super) fn launcher(directories: &[PathBuf]) -> Result<PathBuf> {
    let (path, _) = dekan_inject::ltf::find_launcher_tools(directories)
        .context("Dekan native tools are missing")?;
    let path = path.canonicalize()?;
    let directory = path.parent().context("Launcher directory missing")?;
    dekan_inject::ltf::native_bundle::verify_directory(directory).map_err(anyhow::Error::msg)?;
    Ok(path)
}

pub(super) fn change(launcher: &Path, install: bool) -> Result<bool> {
    let directory = launcher.parent().context("Launcher directory missing")?;
    dekan_inject::ltf::native_bundle::verify_directory(directory).map_err(anyhow::Error::msg)?;
    let current = registration(launcher);
    if current == Registration::Other {
        bail!("Another League client debugger is registered; keeping it unchanged")
    }
    if (install && current == Registration::Ours) || (!install && current == Registration::Missing)
    {
        return Ok(true);
    }
    let argument = if install {
        "--install-ux-shim"
    } else {
        "--remove-ux-shim"
    };
    if !dekan_platform::shell::run_elevated_wait(launcher, argument)? {
        return Ok(false);
    }
    let expected = if install {
        Registration::Ours
    } else {
        Registration::Missing
    };
    Ok(registration(launcher) == expected)
}

pub(super) fn client_endpoint() -> Option<(u32, u16)> {
    let pid = dekan_platform::process::ProcessFinder::find_process_by_name("LeagueClientUx.exe")
        .ok()??;
    let command = dekan_platform::process::query_process_cmdline(pid)?;
    Some((pid, super::cdp::debug_port(&command)?))
}

fn owns(value: &str, launcher: &Path) -> bool {
    let Some(path) = value
        .strip_suffix(" --ux-shim")
        .and_then(|s| s.strip_prefix('"'))
        .and_then(|s| s.strip_suffix('"'))
    else {
        return false;
    };
    match (Path::new(path).canonicalize(), launcher.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
