use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ring::rand::SecureRandom;
use serde_json::{Value, json};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExA, RegDeleteValueA, RegOpenKeyExA, RegSetValueExA,
};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::core::{BOOL, s};

#[link(name = "user32")]
unsafe extern "system" {
    fn GetProcessWindowStation() -> HANDLE;
    fn GetThreadDesktop(dwThreadId: u32) -> HANDLE;
    fn GetUserObjectInformationW(
        hObj: HANDLE,
        nIndex: i32,
        pvInfo: *mut core::ffi::c_void,
        nLength: u32,
        lpnLengthNeeded: *mut u32,
    ) -> BOOL;
}

unsafe fn user_object_name(handle: HANDLE) -> Option<String> {
    if handle.0.is_null() {
        return None;
    }
    let mut buf = [0u16; 256];
    let mut needed = 0u32;
    let ok = unsafe {
        GetUserObjectInformationW(
            handle,
            2,
            buf.as_mut_ptr().cast(),
            (buf.len() * 2) as u32,
            &mut needed,
        )
    };
    if !ok.as_bool() {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub fn current_desktop_path() -> Option<String> {
    unsafe {
        let winsta = user_object_name(GetProcessWindowStation())?;
        let desktop = user_object_name(GetThreadDesktop(GetCurrentThreadId()))?;
        Some(format!("{winsta}\\{desktop}"))
    }
}

use crate::cdp::{cdp_available, get_cdp_port};
use crate::commands::{
    cmd_health, cmd_health_check_off, cmd_import, cmd_kill, cmd_library_call, cmd_list,
    cmd_patcher_status, cmd_spoof_health, cmd_start_patcher, cmd_status, cmd_stop_patcher,
    cmd_toggle, cmd_uninstall,
};

static API_PORT: AtomicU16 = AtomicU16::new(0);
static API_TOKEN: RwLock<String> = RwLock::new(String::new());

pub fn get_api_port() -> u16 {
    API_PORT.load(Ordering::SeqCst)
}

pub fn generate_api_token() -> String {
    let mut raw = [0u8; 16];
    let _ = ring::rand::SystemRandom::new().fill(&mut raw); // ignore-ok: token rng
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    if let Ok(mut lock) = API_TOKEN.write() {
        *lock = token.clone();
    }
    token
}

pub fn token_matches(given: &str) -> bool {
    let expected = match API_TOKEN.read() {
        Ok(t) => t.clone(),
        Err(_) => return false,
    };
    if expected.is_empty() || given.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in given.bytes().zip(expected.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

pub fn broadcast_port(port: u16, token: &str) {
    let mut hkey = HKEY::default();
    let subkey = s!("Software\\DekanBridge");
    if unsafe {
        RegCreateKeyExA(
            HKEY_CURRENT_USER,
            subkey,
            None,
            windows::core::PCSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
    }
    .is_ok()
    {
        let port_val = port as u32;
        let port_bytes =
            unsafe { std::slice::from_raw_parts((&port_val as *const u32).cast::<u8>(), 4) };
        let _ = unsafe { RegSetValueExA(hkey, s!("ApiPort"), None, REG_DWORD, Some(port_bytes)) }; // ignore-ok: reg port
        let mut token_val = token.as_bytes().to_vec();
        token_val.push(0);
        let _ = unsafe { RegSetValueExA(hkey, s!("ApiToken"), None, REG_SZ, Some(&token_val)) }; // ignore-ok: reg token
        let _ = unsafe { RegCloseKey(hkey) }; // ignore-ok: reg close
    }

    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        let dir = PathBuf::from(local_app).join("DekanBridge");
        let _ = std::fs::create_dir_all(&dir); // ignore-ok: create bridge dir
        let _ = std::fs::write(dir.join("port.txt"), format!("{port}\n")); // ignore-ok: write port
        let _ = std::fs::write(dir.join("token.txt"), format!("{token}\n")); // ignore-ok: write token
    }
    let _ = std::fs::write(r"C:\Users\Public\DekanBridge.port", format!("{port}\n")); // ignore-ok: write public port
}

pub fn clear_port_broadcast() {
    let mut hkey = HKEY::default();
    if unsafe {
        RegOpenKeyExA(
            HKEY_CURRENT_USER,
            s!("Software\\DekanBridge"),
            None,
            KEY_WRITE,
            &mut hkey,
        )
    }
    .is_ok()
    {
        let _ = unsafe { RegDeleteValueA(hkey, s!("ApiPort")) }; // ignore-ok: delete reg
        let _ = unsafe { RegDeleteValueA(hkey, s!("ApiToken")) }; // ignore-ok: delete reg
        let _ = unsafe { RegCloseKey(hkey) }; // ignore-ok: reg close
    }

    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        let dir = PathBuf::from(local_app).join("DekanBridge");
        let _ = std::fs::remove_file(dir.join("port.txt")); // ignore-ok: rm port
        let _ = std::fs::remove_file(dir.join("token.txt")); // ignore-ok: rm token
    }
    let _ = std::fs::remove_file(r"C:\Users\Public\DekanBridge.port"); // ignore-ok: rm public port
}

fn handle_command(line: &str, port: u16) -> Value {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return json!({ "ok": false, "error": "invalid JSON" }),
    };

    let cmd = req.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
    let pid = unsafe { GetCurrentProcessId() };

    if cmd == "hello" {
        return json!({
            "ok": true,
            "service": "DekanBridge",
            "version": "1.1-fantome-processor",
            "pid": pid,
            "port": port,
            "desktop": current_desktop_path(),
            "auth": true,
            "credit": "made by lol123love for educational purposes and bcz love loves Mod Manager :p BIG FLOP!"
        });
    }

    if cmd == "ping" {
        let pong = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        return json!({ "ok": true, "pong": pong });
    }

    let token = req.get("token").and_then(|v| v.as_str()).unwrap_or("");
    if !token_matches(token) {
        return json!({ "ok": false, "error": "unauthorized: missing or wrong token" });
    }

    match cmd {
        "status" => cmd_status(port),
        "list" => cmd_list(),
        "import" => cmd_import(&req),
        "toggle" => cmd_toggle(&req),
        "uninstall" => cmd_uninstall(&req),
        "health" => cmd_health(&req),
        "spoof_health" => cmd_spoof_health(),
        "health_check_off" => cmd_health_check_off(),
        "check_mod_health" => cmd_library_call("plugin:library|check_mod_health", &req),
        "repair_mod" => cmd_library_call("plugin:library|repair_mod", &req),
        "start_patcher" => cmd_start_patcher(),
        "stop_patcher" => cmd_stop_patcher(),
        "patcher_status" => cmd_patcher_status(),
        "kill" => cmd_kill(),
        other => json!({ "ok": false, "error": format!("unknown: {other}") }),
    }
}

fn handle_client(mut stream: TcpStream, port: u16) {
    let _ = stream.set_nonblocking(false); // ignore-ok: accepted socket must be blocking on Windows
    let _ = stream.set_read_timeout(Some(Duration::from_secs(300))); // ignore-ok: timeout
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30))); // ignore-ok: timeout
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let mut line = String::new();
    while let Ok(n) = reader.read_line(&mut line) {
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let resp = handle_command(trimmed, port);
            let mut out = serde_json::to_vec(&resp).unwrap_or_default();
            out.push(b'\n');
            if stream.write_all(&out).is_err() {
                break;
            }
        }
        line.clear();
    }
}

pub fn spawn_tcp_server(stop_flag: &'static AtomicBool) {
    std::thread::spawn(move || {
        let mut chosen_port = None;
        let mut listener = None;

        let env_port = std::env::var("LTK_API_PORT")
            .or_else(|_| std::env::var("CELESTIAL_API_PORT"))
            .ok()
            .and_then(|p| p.trim().parse::<u16>().ok());

        let (range_start, range_end) = match env_port {
            Some(p) if p > 0 => (p, p),
            _ => (46321, 46340),
        };

        for p in range_start..=range_end {
            if let Ok(l) = TcpListener::bind(("127.0.0.1", p)) {
                chosen_port = Some(p);
                listener = Some(l);
                break;
            }
        }

        let Some(port) = chosen_port else {
            return;
        };
        let Some(listener) = listener else {
            return;
        };

        let _ = listener.set_nonblocking(true); // ignore-ok: nonblocking set
        API_PORT.store(port, Ordering::SeqCst);
        let token = generate_api_token();
        broadcast_port(port, &token);

        while !stop_flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    std::thread::spawn(move || {
                        handle_client(stream, port);
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    break;
                }
            }
        }
        clear_port_broadcast();
        API_PORT.store(0, Ordering::SeqCst);
    });
}

pub fn spawn_auto_spoof_thread(stop_flag: &'static AtomicBool) {
    std::thread::spawn(move || {
        while !stop_flag.load(Ordering::Relaxed) {
            let cdp_port = get_cdp_port();
            if cdp_available(cdp_port) {
                let res = cmd_spoof_health();
                let is_ok = res.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                if is_ok {
                    let _ = cmd_health_check_off(); // ignore-ok: best effort health check disable
                    for _ in 0..50 {
                        if stop_flag.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    continue;
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    static TOKEN_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn public_commands_succeed_without_token() {
        let hello = handle_command(r#"{"cmd":"hello"}"#, 46321);
        assert_eq!(hello["ok"], true);
        assert_eq!(hello["service"], "DekanBridge");
        assert_eq!(hello["port"], 46321);
        assert!(
            hello["desktop"]
                .as_str()
                .is_some_and(|d| !d.trim().is_empty())
        );

        let ping = handle_command(r#"{"cmd":"ping"}"#, 46321);
        assert_eq!(ping["ok"], true);
        assert!(ping["pong"].is_u64());
    }

    #[test]
    fn protected_commands_require_matching_token() {
        let _guard = TOKEN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let token = generate_api_token();
        assert_eq!(token.len(), 32);

        let rejected = handle_command(r#"{"cmd":"status","token":"wrong"}"#, 46321);
        assert_eq!(rejected["ok"], false);
        assert!(
            rejected["error"]
                .as_str()
                .unwrap_or("")
                .contains("unauthorized")
        );

        let accepted = handle_command(
            &format!(r#"{{"cmd":"health","modId":"m1","token":"{token}"}}"#),
            46321,
        );
        assert_eq!(accepted["ok"], true);
        assert_eq!(accepted["value"]["m1"]["health"], "healthy");
    }

    #[test]
    fn invalid_json_or_unknown_command_returns_error() {
        let _guard = TOKEN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let token = generate_api_token();
        let bad_json = handle_command("{not-json", 46321);
        assert_eq!(bad_json["ok"], false);

        let unknown = handle_command(&format!(r#"{{"cmd":"bogus","token":"{token}"}}"#), 46321);
        assert_eq!(unknown["ok"], false);
        assert!(unknown["error"].as_str().unwrap_or("").contains("unknown"));
    }
}
