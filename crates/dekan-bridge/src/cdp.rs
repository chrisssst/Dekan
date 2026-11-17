use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::prelude::*;
use ring::rand::SecureRandom;
use serde_json::Value;

static CDP_PORT: AtomicU16 = AtomicU16::new(9222);
static CDP_REQ_ID: AtomicU64 = AtomicU64::new(1);
static CDP_MUTEX: Mutex<()> = Mutex::new(());
static WS_SESSION: Mutex<Option<WsConnection>> = Mutex::new(None);

pub fn set_cdp_port(port: u16) {
    CDP_PORT.store(port, Ordering::SeqCst);
}

pub fn get_cdp_port() -> u16 {
    CDP_PORT.load(Ordering::SeqCst)
}

pub fn setup_cdp_port() -> u16 {
    if let Ok(val) = std::env::var("LTF_CDP_PORT") {
        if let Ok(p) = val.trim().parse::<u16>() {
            if p > 0 {
                set_cdp_port(p);
                return p;
            }
        }
    }

    if let Ok(val) = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
        if let Some(pos) = val.find("--remote-debugging-port=") {
            let rem = &val[pos + 24..];
            let digits: String = rem.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(p) = digits.parse::<u16>() {
                if p > 0 {
                    set_cdp_port(p);
                    return p;
                }
            }
        }
    }

    let mut raw = [0u8; 2];
    let _ = ring::rand::SystemRandom::new().fill(&mut raw); // ignore-ok: fallback random
    let rand_val = u16::from_le_bytes(raw);
    let port = 20000 + (rand_val % 20000);

    let own_args =
        format!("--remote-debugging-port={port} --remote-allow-origins=http://127.0.0.1:{port}");
    let new_args = match std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
        Ok(existing) if !existing.is_empty() => format!("{existing} {own_args}"),
        _ => own_args,
    };
    unsafe {
        use windows::Win32::System::Environment::SetEnvironmentVariableW;
        use windows::core::PCWSTR;
        let key: Vec<u16> = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS\0"
            .encode_utf16()
            .collect();
        let val_w: Vec<u16> = format!("{new_args}\0").encode_utf16().collect();
        let _ = SetEnvironmentVariableW(PCWSTR(key.as_ptr()), PCWSTR(val_w.as_ptr())); // ignore-ok: env var
    }

    set_cdp_port(port);
    port
}

pub fn http_get(port: u16, path: &str, timeout: Duration) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;

    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).ok()?;

    let mut resp = Vec::new();
    let mut buf = [0u8; 4096];
    let mut expected_len: Option<usize> = None;
    let mut body_start: Option<usize> = None;

    while let Ok(n) = stream.read(&mut buf) {
        if n == 0 {
            break;
        }
        resp.extend_from_slice(&buf[..n]);

        if body_start.is_none() {
            if let Some(pos) = resp.windows(4).position(|w| w == b"\r\n\r\n") {
                body_start = Some(pos + 4);
                let header_str = String::from_utf8_lossy(&resp[..pos]).to_lowercase();
                for line in header_str.lines() {
                    if let Some(val) = line.strip_prefix("content-length:") {
                        expected_len = val.trim().parse::<usize>().ok();
                    }
                }
            }
        }

        if let (Some(start), Some(len)) = (body_start, expected_len) {
            if resp.len() >= start + len {
                break;
            }
        }
    }

    let start = body_start?;
    String::from_utf8(resp[start..].to_vec()).ok()
}

pub fn cdp_available(port: u16) -> bool {
    http_get(port, "/json/version", Duration::from_millis(500)).is_some()
}

pub fn cdp_find_ws_url(port: u16) -> Option<String> {
    let body = http_get(port, "/json", Duration::from_millis(1500))?;
    let val: Value = serde_json::from_str(&body).ok()?;
    let arr = val.as_array()?;

    for item in arr {
        let is_page = item.get("type").and_then(|v| v.as_str()) == Some("page");
        if !is_page {
            continue;
        }
        let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.contains("tauri.localhost") {
            if let Some(ws_url) = item.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                return Some(ws_url.to_string());
            }
        }
    }

    None
}

struct WsConnection {
    stream: TcpStream,
}

impl WsConnection {
    fn connect(ws_url: &str, cdp_port: u16) -> Result<Self, String> {
        let stripped = ws_url.strip_prefix("ws://").ok_or("invalid ws url")?;
        let (host_port, path) = match stripped.find('/') {
            Some(idx) => (&stripped[..idx], &stripped[idx..]),
            None => (stripped, "/"),
        };
        let (host, port) = match host_port.find(':') {
            Some(idx) => (
                &host_port[..idx],
                host_port[idx + 1..]
                    .parse::<u16>()
                    .map_err(|e| e.to_string())?,
            ),
            None => (host_port, cdp_port),
        };

        let mut stream = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_millis(10000)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_millis(5000)))
            .map_err(|e| e.to_string())?;

        let mut raw_key = [0u8; 16];
        let _ = ring::rand::SystemRandom::new().fill(&mut raw_key); // ignore-ok: sec random
        let sec_key = BASE64_STANDARD.encode(raw_key);
        let origin = format!("http://127.0.0.1:{cdp_port}");

        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nOrigin: {origin}\r\nSec-WebSocket-Key: {sec_key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream
            .write_all(req.as_bytes())
            .map_err(|e| e.to_string())?;

        let mut hdr = Vec::new();
        let mut b = [0u8; 1];
        while !hdr.windows(4).any(|w| w == b"\r\n\r\n") {
            if stream.read_exact(&mut b).is_err() {
                return Err("ws handshake failed".into());
            }
            hdr.push(b[0]);
            if hdr.len() > 8192 {
                return Err("ws handshake header too long".into());
            }
        }

        let hdr_str = String::from_utf8_lossy(&hdr);
        if !hdr_str.contains(" 101 ") && !hdr_str.contains(" 101\r") {
            return Err("ws handshake refused".into());
        }

        Ok(Self { stream })
    }

    fn send_text(&mut self, text: &str) -> Result<(), String> {
        let mut frame = Vec::new();
        frame.push(0x81);
        let len = text.len();
        if len < 126 {
            frame.push(0x80 | (len as u8));
        } else if len < 65536 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }

        let mut mask = [0u8; 4];
        let _ = ring::rand::SystemRandom::new().fill(&mut mask); // ignore-ok: mask key
        frame.extend_from_slice(&mask);

        let masked_payload: Vec<u8> = text
            .as_bytes()
            .iter()
            .enumerate()
            .map(|(i, &b)| b ^ mask[i % 4])
            .collect();
        frame.extend_from_slice(&masked_payload);

        self.stream.write_all(&frame).map_err(|e| e.to_string())
    }

    fn recv_text(&mut self, deadline: Instant) -> Result<String, String> {
        let mut message = Vec::new();
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err("cdp evaluate timed out".into());
            }
            let remaining = deadline - now;
            let timeout = Some(remaining.max(Duration::from_millis(500)));
            let _ = self.stream.set_read_timeout(timeout); // ignore-ok: timeout set

            let mut hdr = [0u8; 2];
            self.stream
                .read_exact(&mut hdr)
                .map_err(|e| e.to_string())?;

            let fin = (hdr[0] & 0x80) != 0;
            let opcode = hdr[0] & 0x0F;
            let masked = (hdr[1] & 0x80) != 0;
            let mut len = (hdr[1] & 0x7F) as usize;

            if len == 126 {
                let mut ext = [0u8; 2];
                self.stream
                    .read_exact(&mut ext)
                    .map_err(|e| e.to_string())?;
                len = u16::from_be_bytes(ext) as usize;
            } else if len == 127 {
                let mut ext = [0u8; 8];
                self.stream
                    .read_exact(&mut ext)
                    .map_err(|e| e.to_string())?;
                len = u64::from_be_bytes(ext) as usize;
            }

            let mut mask = [0u8; 4];
            if masked {
                self.stream
                    .read_exact(&mut mask)
                    .map_err(|e| e.to_string())?;
            }

            let mut payload = vec![0u8; len];
            self.stream
                .read_exact(&mut payload)
                .map_err(|e| e.to_string())?;
            if masked {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
            }

            if opcode == 0x8 {
                return Err("ws closed by peer".into());
            }
            if opcode == 0x9 {
                let mut pong = vec![0x8A, 0x80];
                let mut pmask = [0u8; 4];
                let _ = ring::rand::SystemRandom::new().fill(&mut pmask); // ignore-ok: pong mask
                pong.extend_from_slice(&pmask);
                let _ = self.stream.write_all(&pong); // ignore-ok: pong send
                continue;
            }

            message.extend_from_slice(&payload);
            if fin {
                return String::from_utf8(message).map_err(|e| e.to_string());
            }
        }
    }
}

pub fn cdp_evaluate_raw(js_expr: &str, timeout_ms: u64) -> Result<Value, String> {
    let port = get_cdp_port();
    let _lock = CDP_MUTEX
        .lock()
        .map_err(|_| "cdp lock poisoned".to_string())?;

    let req_id = CDP_REQ_ID.fetch_add(1, Ordering::SeqCst);
    let payload = serde_json::json!({
        "id": req_id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": js_expr,
            "awaitPromise": true,
            "returnByValue": true
        }
    });
    let payload_str = payload.to_string();

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut ws_guard = WS_SESSION
        .lock()
        .map_err(|_| "ws session lock poisoned".to_string())?;

    for attempt in 0..2 {
        if ws_guard.is_none() {
            let ws_url = cdp_find_ws_url(port).ok_or("tauri page ws url not found")?;
            let conn = WsConnection::connect(&ws_url, port)?;
            *ws_guard = Some(conn);
        }

        if let Some(conn) = ws_guard.as_mut() {
            if conn.send_text(&payload_str).is_err() {
                *ws_guard = None;
                if attempt == 0 {
                    continue;
                }
                return Err("ws send failed".into());
            }

            let recv_err;
            loop {
                match conn.recv_text(deadline) {
                    Ok(frame) => {
                        if let Ok(val) = serde_json::from_str::<Value>(&frame) {
                            if val.get("id").and_then(|v| v.as_u64()) == Some(req_id) {
                                if let Some(res_val) = val
                                    .get("result")
                                    .and_then(|r| r.get("result"))
                                    .and_then(|inner| inner.get("value"))
                                {
                                    return Ok(res_val.clone());
                                }
                                return Ok(val);
                            }
                        }
                    }
                    Err(e) => {
                        recv_err = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = recv_err {
                *ws_guard = None;
                if attempt == 0 {
                    continue;
                }
                return Err(e);
            }
        }
    }

    Err("cdp evaluate failed".into())
}

pub fn tauri_invoke(command: &str, args: &Value, timeout_ms: u64) -> Value {
    let cmd_json = serde_json::to_string(command).unwrap_or_default();
    let args_json = serde_json::to_string(args).unwrap_or_default();
    let js = format!(
        "(async () => {{ try {{ const r = await window.__TAURI_INTERNALS__.invoke({cmd_json}, {args_json}); return r; }} catch (e) {{ return {{ ok: false, error: (e && e.message) ? e.message : String(e) }}; }} }})()"
    );

    match cdp_evaluate_raw(&js, timeout_ms) {
        Ok(v) => v,
        Err(e) => serde_json::json!({ "ok": false, "error": e }),
    }
}

pub fn close_cdp() {
    let _ = WS_SESSION.lock().map(|mut g| *g = None); // ignore-ok: close ws
}
