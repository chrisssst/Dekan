use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::discovery::BridgeEndpoint;
use crate::error::InjectError;

pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
pub const FAST_TIMEOUT: Duration = Duration::from_secs(5);
pub const IMPORT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone)]
pub struct BridgeClient {
    endpoint: BridgeEndpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerModInfo {
    pub id: String,
    pub name: Option<String>,
    pub enabled: Option<bool>,
}

impl BridgeClient {
    #[must_use]
    pub fn new(endpoint: BridgeEndpoint) -> Self {
        Self { endpoint }
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.endpoint.port
    }

    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.endpoint.pid
    }

    pub async fn send_raw(
        &self,
        payload: &serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, InjectError> {
        let connect = TcpStream::connect(("127.0.0.1", self.endpoint.port));
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| InjectError::Process("DekanBridge connect timeout".into()))?
            .map_err(|e| InjectError::Process(format!("DekanBridge connection refused: {e}")))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let mut data = serde_json::to_vec(payload)
            .map_err(|e| InjectError::Process(format!("JSON serialization error: {e}")))?;
        data.push(b'\n');

        let write_op = writer.write_all(&data);
        tokio::time::timeout(timeout, write_op)
            .await
            .map_err(|_| InjectError::Process("DekanBridge write timeout".into()))?
            .map_err(|e| InjectError::Process(format!("DekanBridge write failed: {e}")))?;

        let mut line = String::new();
        let read_op = reader.read_line(&mut line);
        tokio::time::timeout(timeout, read_op)
            .await
            .map_err(|_| InjectError::Process("DekanBridge response timeout".into()))?
            .map_err(|e| InjectError::Process(format!("DekanBridge read failed: {e}")))?;

        if line.is_empty() {
            return Err(InjectError::Process(
                "DekanBridge closed connection without response".into(),
            ));
        }

        let resp: serde_json::Value = serde_json::from_str(&line)
            .map_err(|e| InjectError::Process(format!("Invalid response JSON from bridge: {e}")))?;

        Ok(resp)
    }

    pub async fn request(
        &self,
        mut payload: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, InjectError> {
        if let Some(cmd) = payload.get("cmd").and_then(|c| c.as_str()) {
            if cmd != "hello" && cmd != "ping" {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert(
                        "token".into(),
                        serde_json::Value::String(self.endpoint.token.clone()),
                    );
                }
            }
        }

        let resp = self.send_raw(&payload, timeout).await?;
        let fmt_err = |err: Option<&serde_json::Value>, fallback: &str| -> String {
            match err {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(v) if !v.is_null() => v.to_string(),
                _ => fallback.to_string(),
            }
        };
        if resp.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let err_msg = fmt_err(
                resp.get("error")
                    .or_else(|| resp.get("response").and_then(|r| r.get("error"))),
                "unknown error",
            );
            return Err(InjectError::Process(format!(
                "DekanBridge error: {err_msg}"
            )));
        }
        if let Some(inner) = resp.get("response") {
            if inner.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                let err_msg = fmt_err(inner.get("error"), "unknown inner error");
                return Err(InjectError::Process(format!(
                    "DekanBridge error: {err_msg}"
                )));
            }
        }

        Ok(resp)
    }

    pub async fn ping(&self) -> Result<(), InjectError> {
        let payload = serde_json::json!({ "cmd": "ping" });
        self.request(payload, FAST_TIMEOUT).await.map(|_| ())
    }

    pub async fn status(&self) -> Result<serde_json::Value, InjectError> {
        let payload = serde_json::json!({ "cmd": "status" });
        self.request(payload, FAST_TIMEOUT).await
    }

    pub async fn health_check_off(&self) -> Result<(), InjectError> {
        let payload = serde_json::json!({ "cmd": "health_check_off" });
        self.request(payload, DEFAULT_TIMEOUT).await.map(|_| ())
    }

    pub async fn list_mods(&self) -> Result<Vec<ManagerModInfo>, InjectError> {
        let payload = serde_json::json!({ "cmd": "list" });
        let resp = self.request(payload, DEFAULT_TIMEOUT).await?;
        let library = resp
            .get("library")
            .or_else(|| resp.get("response").and_then(|r| r.get("value")))
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                InjectError::Process("Invalid library format returned by bridge".into())
            })?;

        let mut mods = Vec::new();
        for item in library {
            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                let name = item
                    .get("displayName")
                    .or_else(|| item.get("name"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                let enabled = item.get("enabled").and_then(|v| v.as_bool());
                mods.push(ManagerModInfo {
                    id: id.to_owned(),
                    name,
                    enabled,
                });
            }
        }
        Ok(mods)
    }

    pub async fn import_archive(
        &self,
        path: &Path,
        name: Option<&str>,
    ) -> Result<String, InjectError> {
        let path_str = path.to_string_lossy().to_string();
        let mut payload = serde_json::json!({
            "cmd": "import",
            "path": path_str
        });
        if let Some(n) = name {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("name".into(), serde_json::Value::String(n.into()));
            }
        }

        let resp = self.request(payload, IMPORT_TIMEOUT).await?;
        Ok(extract_imported_mod_id(&resp))
    }

    pub async fn import_bytes(&self, data: &[u8], name: &str) -> Result<String, InjectError> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        let payload = serde_json::json!({
            "cmd": "import",
            "data_b64": encoded,
            "name": name
        });
        let resp = self.request(payload, IMPORT_TIMEOUT).await?;
        Ok(extract_imported_mod_id(&resp))
    }

    pub async fn toggle_mod(&self, target: &str, enabled: bool) -> Result<(), InjectError> {
        let payload = serde_json::json!({
            "cmd": "toggle",
            "target": target,
            "enabled": enabled
        });
        self.request(payload, DEFAULT_TIMEOUT).await.map(|_| ())
    }

    pub async fn uninstall_mod(&self, target: &str) -> Result<(), InjectError> {
        let payload = serde_json::json!({
            "cmd": "uninstall",
            "target": target
        });
        self.request(payload, DEFAULT_TIMEOUT).await.map(|_| ())
    }

    pub async fn start_patcher(&self) -> Result<(), InjectError> {
        let payload = serde_json::json!({ "cmd": "start_patcher" });
        self.request(payload, DEFAULT_TIMEOUT).await.map(|_| ())
    }

    pub async fn stop_patcher(&self) -> Result<(), InjectError> {
        let payload = serde_json::json!({ "cmd": "stop_patcher" });
        self.request(payload, DEFAULT_TIMEOUT).await.map(|_| ())
    }

    pub async fn patcher_status(&self) -> Result<serde_json::Value, InjectError> {
        let payload = serde_json::json!({ "cmd": "patcher_status" });
        self.request(payload, DEFAULT_TIMEOUT).await
    }

    pub async fn kill(&self) -> Result<(), InjectError> {
        let payload = serde_json::json!({ "cmd": "kill" });
        self.request(payload, FAST_TIMEOUT).await.map(|_| ())
    }
}

#[must_use]
pub fn extract_imported_mod_id(resp: &serde_json::Value) -> String {
    resp.get("id")
        .or_else(|| resp.get("modId"))
        .or_else(|| {
            resp.get("response")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.get("mod"))
                .and_then(|m| m.get("id"))
        })
        .or_else(|| {
            resp.get("response")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.get("id"))
        })
        .or_else(|| resp.get("response").and_then(|r| r.get("id")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

#[must_use]
pub fn extract_patcher_state(resp: &serde_json::Value) -> (bool, bool, Option<String>) {
    let inner = resp
        .get("response")
        .and_then(|r| r.get("value"))
        .or_else(|| resp.get("response"))
        .unwrap_or(resp);
    let running = inner
        .get("running")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let phase = inner
        .get("phase")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let attached = inner
        .get("attached")
        .and_then(|v| v.as_bool())
        .unwrap_or(matches!(
            phase.as_deref(),
            Some(
                "injected"
                    | "attached"
                    | "overlayActive"
                    | "waiting"
                    | "awaitingExit"
                    | "waitingForExit"
            )
        ));
    (running, attached, phase)
}

#[cfg(test)]
mod tests;
