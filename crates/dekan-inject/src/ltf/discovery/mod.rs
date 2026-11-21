use std::path::PathBuf;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::warn;

pub const PORT_RANGE_START: u16 = 46321;
pub const PORT_RANGE_END: u16 = 46340;
pub const FAST_PROBE_TIMEOUT: Duration = Duration::from_millis(150);

#[derive(Debug, Clone)]
pub struct BridgeEndpoint {
    pub port: u16,
    pub token: String,
    pub pid: Option<u32>,
}

#[must_use]
pub fn bridge_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local_app).join("BulletBridge");
        dirs.push(p);
    }
    dirs.push(PathBuf::from(r"C:\Users\Public"));
    dirs
}

pub fn read_published_token() -> Option<String> {
    for dir in bridge_dirs() {
        let token_file = dir.join("token.txt");
        if let Ok(content) = std::fs::read_to_string(&token_file) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

pub fn read_published_port() -> Option<u16> {
    for dir in bridge_dirs() {
        for file_name in ["port.txt", "BulletBridge.port"] {
            let port_file = dir.join(file_name);
            if let Ok(content) = std::fs::read_to_string(&port_file) {
                if let Ok(port) = content.trim().parse::<u16>() {
                    if (PORT_RANGE_START..=PORT_RANGE_END).contains(&port) {
                        return Some(port);
                    }
                }
            }
        }
    }
    None
}

struct ProbedBridge {
    port: u16,
    pid: Option<u32>,
    desktop: Option<String>,
}

async fn probe_port_info(port: u16, timeout: Duration) -> Option<ProbedBridge> {
    let connect = TcpStream::connect(("127.0.0.1", port));
    let mut stream = tokio::time::timeout(timeout, connect).await.ok()?.ok()?;

    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);

    let hello_cmd = b"{\"cmd\":\"hello\"}\n";
    tokio::time::timeout(timeout, writer.write_all(hello_cmd))
        .await
        .ok()?
        .ok()?;

    let mut line = String::new();
    tokio::time::timeout(timeout, reader.read_line(&mut line))
        .await
        .ok()?
        .ok()?;

    let val: serde_json::Value = serde_json::from_str(&line).ok()?;
    if val.get("ok").and_then(|v| v.as_bool()) == Some(true)
        && val.get("service").and_then(|v| v.as_str()) == Some("BulletBridge")
    {
        let pid = val.get("pid").and_then(|v| v.as_u64()).map(|p| p as u32);
        let desktop = val
            .get("desktop")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        Some(ProbedBridge { port, pid, desktop })
    } else {
        None
    }
}

pub async fn probe_port(port: u16, timeout: Duration) -> Option<(u16, Option<u32>)> {
    probe_port_info(port, timeout)
        .await
        .map(|info| (info.port, info.pid))
}

fn effective_desktop(info: &ProbedBridge) -> Option<String> {
    if let Some(ref d) = info.desktop {
        return Some(d.clone());
    }
    #[cfg(windows)]
    if let Some(pid) = info.pid {
        return dekan_platform::process::process_desktop(pid);
    }
    None
}

#[cfg_attr(not(windows), allow(unused_variables))]
fn is_non_default_desktop(info: &ProbedBridge) -> bool {
    #[cfg(windows)]
    if let Some(desktop) = effective_desktop(info) {
        return !dekan_platform::process::is_default_desktop(&desktop);
    }
    false
}

async fn kill_wrong_desktop_bridge(info: &ProbedBridge, token: &str) {
    let desktop = effective_desktop(info);
    warn!(
        port = info.port,
        pid = ?info.pid,
        desktop = ?desktop,
        "DekanBridge is running on a non-Default desktop and cannot detect League of Legends; closing it to relaunch on WinSta0\\Default"
    );
    if !token.is_empty() {
        if let Ok(Ok(mut stream)) = tokio::time::timeout(
            Duration::from_millis(500),
            TcpStream::connect(("127.0.0.1", info.port)),
        )
        .await
        {
            let payload = serde_json::json!({ "cmd": "kill", "token": token });
            if let Ok(mut bytes) = serde_json::to_vec(&payload) {
                bytes.push(b'\n');
                let _ =
                    tokio::time::timeout(Duration::from_secs(1), stream.write_all(&bytes)).await; // ignore-ok: best-effort kill command
            }
        }
    }
    if let Some(pid) = info.pid {
        for _ in 0..20 {
            if dekan_platform::process::ProcessFinder::get_process_path(pid)
                .ok()
                .flatten()
                .is_none()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        #[cfg(windows)]
        let _ = dekan_platform::process::terminate_process(pid); // ignore-ok: force-close non-default desktop bridge
    }
}

pub async fn discover_bridge(expected_pid: Option<u32>) -> Option<BridgeEndpoint> {
    let token = read_published_token().unwrap_or_default();

    if let Some(port) = read_published_port() {
        if let Some(info) = probe_port_info(port, FAST_PROBE_TIMEOUT).await {
            if expected_pid.is_none() || info.pid == expected_pid {
                if expected_pid.is_none() && is_non_default_desktop(&info) {
                    kill_wrong_desktop_bridge(&info, &token).await;
                    return None;
                }
                return Some(BridgeEndpoint {
                    port: info.port,
                    token,
                    pid: info.pid,
                });
            }
        }
    }

    if !is_manager_running() {
        return None;
    }

    let mut set = tokio::task::JoinSet::new();
    for port in PORT_RANGE_START..=PORT_RANGE_END {
        set.spawn(async move { probe_port_info(port, FAST_PROBE_TIMEOUT).await });
    }
    while let Some(res) = set.join_next().await {
        if let Ok(Some(info)) = res {
            if expected_pid.is_none() || info.pid == expected_pid {
                let fresh_token = if token.is_empty() {
                    read_published_token().unwrap_or_default()
                } else {
                    token.clone()
                };
                if expected_pid.is_none() && is_non_default_desktop(&info) {
                    kill_wrong_desktop_bridge(&info, &fresh_token).await;
                    continue;
                }
                return Some(BridgeEndpoint {
                    port: info.port,
                    token: fresh_token,
                    pid: info.pid,
                });
            }
        }
    }

    None
}

fn is_manager_running() -> bool {
    dekan_platform::process::ProcessFinder::find_any_process(&[
        "ltk-manager.exe",
        "LTK Manager.exe",
    ])
    .ok()
    .flatten()
    .is_some()
}

#[cfg(test)]
mod tests;
