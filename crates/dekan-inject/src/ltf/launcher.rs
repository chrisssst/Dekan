use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tracing::{info, warn};

use super::client::BridgeClient;
use super::discovery::discover_bridge;
use crate::error::InjectError;

pub const LAUNCHER_EXE_NAME: &str = "dekan_launcher.exe";
pub const BRIDGE_DLL_NAME: &str = "dekan_bridge.dll";
pub const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
pub const MIN_MANAGER_VERSION: (u16, u16, u16) = (1, 33, 0);

#[derive(Debug, PartialEq, Eq)]
pub enum ManagerInstallState {
    Ready {
        path: PathBuf,
        version: Option<(u16, u16, u16)>,
    },
    Outdated {
        path: PathBuf,
        found: (u16, u16, u16),
    },
    Missing,
}

#[must_use]
pub fn check_manager_install() -> ManagerInstallState {
    let Some(path) = find_manager_manager() else {
        return ManagerInstallState::Missing;
    };
    let version = dekan_platform::version::read_pe_version(&path);
    if let Some(v) = version {
        if !dekan_platform::version::is_version_at_least(v, MIN_MANAGER_VERSION) {
            return ManagerInstallState::Outdated { path, found: v };
        }
    }
    ManagerInstallState::Ready { path, version }
}

#[must_use]
pub fn find_manager_manager() -> Option<PathBuf> {
    let override_path = std::env::var("DEKAN_LTK_MANAGER")
        .or_else(|_| std::env::var("LTK_MANAGER_EXE"))
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_file());
    override_path.or_else(|| {
        dekan_platform::version::discover_installed_exe(
            &["ltk manager", "leaguetoolkit"],
            &["ltk-manager.exe", "LTK Manager.exe"],
        )
    })
}

#[must_use]
pub fn find_launcher_tools(tools_dirs: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    for dir in tools_dirs {
        let launcher = dir.join(LAUNCHER_EXE_NAME);
        let dll = dir.join(BRIDGE_DLL_NAME);
        if launcher.is_file() && dll.is_file() {
            return Some((launcher, dll));
        }
    }
    None
}

pub async fn ensure_bridge_running(
    tools_dirs: &[PathBuf],
    timeout: Duration,
) -> Result<BridgeClient, InjectError> {
    let (launcher_exe, bridge_dll) = find_launcher_tools(tools_dirs).ok_or_else(|| {
        InjectError::Process("Dekan bridge tools are missing; reinstall Dekan 2.3".into())
    })?;
    let tools_dir = launcher_exe
        .parent()
        .ok_or_else(|| InjectError::Process("Dekan launcher has no containing directory".into()))?;
    super::native_bundle::verify_directory(tools_dir).map_err(InjectError::Process)?;
    for path in [&launcher_exe, &bridge_dll] {
        crate::trust::verify_native_file(path)?;
    }

    if let Some(endpoint) = discover_bridge(None).await {
        let client = BridgeClient::new(endpoint);
        if client.ping().await.is_ok() {
            return Ok(client);
        }
    }

    let manager_path = match check_manager_install() {
        ManagerInstallState::Ready { path, version } => {
            if let Some(v) = version {
                info!(
                    version = %dekan_platform::version::format_version(v),
                    "Verified installed Mod Manager version"
                );
            }
            path
        }
        ManagerInstallState::Outdated { path, found } => {
            let found_str = dekan_platform::version::format_version(found);
            let req_str = dekan_platform::version::format_version(MIN_MANAGER_VERSION);
            warn!(
                manager = %path.display(),
                found = %found_str,
                required = %req_str,
                "Mod Manager is outdated"
            );
            return Err(InjectError::Process(format!(
                "Mod Manager version {found_str} is outdated. Please update Mod Manager to {req_str}+ so Dekan works properly."
            )));
        }
        ManagerInstallState::Missing => {
            return Err(InjectError::Process(
                "Mod Manager (ltk-manager.exe) is not installed on this system. Please install Mod Manager."
                    .into(),
            ));
        }
    };

    info!(
        launcher = %launcher_exe.display(),
        manager = %manager_path.display(),
        "Launching Mod Manager with LTF bridge"
    );

    let mut cmd = Command::new(&launcher_exe);
    cmd.arg("--quiet").arg("--exe").arg(&manager_path);
    if let Ok(logs_dir) = dekan_platform::paths::logs_dir() {
        cmd.arg("--log-dir").arg(logs_dir);
    }
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);

    let status = cmd
        .status()
        .await
        .map_err(|e| InjectError::Process(format!("Failed to execute {LAUNCHER_EXE_NAME}: {e}")))?;

    let code = status.code().unwrap_or(-1);
    if code == 5 {
        return Err(InjectError::Process(
            "Mod Manager is already running without Dekan bridge. Please close Mod Manager completely (including system tray) and try again."
                .into(),
        ));
    }
    if code != 0 {
        warn!(exit_code = code, "dekan_launcher exited with non-zero code");
        return Err(InjectError::Process(format!(
            "{LAUNCHER_EXE_NAME} failed to launch Mod Manager (exit code {code})"
        )));
    }

    wait_for_bridge(timeout, None).await
}

pub async fn wait_for_bridge(
    timeout: Duration,
    expected_pid: Option<u32>,
) -> Result<BridgeClient, InjectError> {
    let deadline = Instant::now() + timeout;
    let poll_interval = Duration::from_millis(250);

    while Instant::now() < deadline {
        if let Some(endpoint) = discover_bridge(expected_pid).await {
            let client = BridgeClient::new(endpoint);
            if client.ping().await.is_ok() {
                return Ok(client);
            }
        }
        tokio::time::sleep(poll_interval).await;
    }

    Err(InjectError::Process(format!(
        "Timed out after {}s waiting for DekanBridge to become ready",
        timeout.as_secs()
    )))
}
