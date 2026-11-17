use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::prelude::*;
use serde_json::{Value, json};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{ExitProcess, GetCurrentProcessId};

use crate::cdp::{cdp_available, cdp_evaluate_raw, get_cdp_port, tauri_invoke};

struct HandleGuard(HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.0.is_null() && self.0.0 != -1 as _ {
            let _ = unsafe { CloseHandle(self.0) }; // ignore-ok: close handle
        }
    }
}

fn is_valid_game_dir(p: &Path) -> bool {
    p.join("DATA").join("FINAL").exists()
}

fn check_path_and_sub(p: PathBuf) -> Option<PathBuf> {
    [p.join("Game"), p]
        .into_iter()
        .find(|cand| is_valid_game_dir(cand))
}

pub fn detect_league_game_path(hint: Option<&str>) -> Option<PathBuf> {
    if let Some(h) = hint {
        if let Some(p) = check_path_and_sub(PathBuf::from(h)) {
            return Some(p);
        }
    }
    if let Ok(env_path) = std::env::var("LTK_LEAGUE_PATH") {
        if let Some(p) = check_path_and_sub(PathBuf::from(env_path)) {
            return Some(p);
        }
    }
    let sys_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let inst_path = PathBuf::from(&sys_drive)
        .join("ProgramData")
        .join("Riot Games")
        .join("RiotClientInstalls.json");
    if let Ok(content) = std::fs::read_to_string(&inst_path) {
        if let Ok(val) = serde_json::from_str::<Value>(&content) {
            if let Some(obj) = val.get("associated_client").and_then(|v| v.as_object()) {
                for (k, _) in obj {
                    let lk = k.to_lowercase();
                    if lk.contains("league of legends") && !lk.contains("pbe") {
                        if let Some(p) = check_path_and_sub(PathBuf::from(k)) {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }
    if let Some(p) = unsafe { find_process_game_path() } {
        return Some(p);
    }
    for drive in [r"C:\", r"D:\", r"E:\", r"F:\", r"G:\"] {
        let cand = PathBuf::from(drive)
            .join("Riot Games")
            .join("League of Legends")
            .join("Game");
        if is_valid_game_dir(&cand) {
            return Some(cand);
        }
    }
    None
}

unsafe fn find_process_game_path() -> Option<PathBuf> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };

    let snap_h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
    let _snap_guard = HandleGuard(snap_h);
    let mut pe = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    if Process32FirstW(snap_h, &mut pe).is_err() {
        return None;
    }
    loop {
        let exe = String::from_utf16_lossy(&pe.szExeFile)
            .trim_matches('\0')
            .to_lowercase();
        if (exe == "leagueclientux.exe" || exe == "league of legends.exe")
            && let Ok(proc_h) =
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pe.th32ProcessID)
        {
            let _proc_guard = HandleGuard(proc_h);
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            let dst = windows::core::PWSTR(buf.as_mut_ptr());
            if QueryFullProcessImageNameW(proc_h, PROCESS_NAME_FORMAT(0), dst, &mut size).is_ok()
                && let Some(parent) =
                    PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])).parent()
                && let Some(valid) = check_path_and_sub(parent.to_path_buf())
            {
                return Some(valid);
            }
        }
        if Process32NextW(snap_h, &mut pe).is_err() {
            break;
        }
    }
    None
}

pub fn cmd_status(tcp_port: u16) -> Value {
    let pid = unsafe { GetCurrentProcessId() };
    let cdp_port = get_cdp_port();
    let game_path = detect_league_game_path(None)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    json!({
        "ok": true,
        "pid": pid,
        "cdp": { "port": cdp_port, "available": cdp_available(cdp_port) },
        "api": { "tcp_port": tcp_port, "tcp_ok": tcp_port != 0 },
        "league_game_path": game_path
    })
}

pub fn cmd_list() -> Value {
    let resp = tauri_invoke("plugin:library|get_installed_mods", &json!({}), 15000);
    let is_ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    let library = resp.get("value").cloned().unwrap_or_else(|| json!([]));
    json!({ "ok": is_ok, "library": library, "raw": resp })
}

fn sanitize_staged_name(name: &str) -> String {
    let mut clean: String = name
        .chars()
        .map(|c| {
            if c < ' ' || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    while clean.ends_with('.') || clean.ends_with(' ') {
        clean.pop();
    }
    if clean.is_empty() {
        clean = "ImportedMod".into();
    }
    let lower = clean.to_lowercase();
    if !lower.ends_with(".fantome") && !lower.ends_with(".modpkg") {
        clean.push_str(".fantome");
    }
    clean
}

pub fn cmd_import(req: &Value) -> Value {
    let path_val = req.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let data_b64 = req.get("data_b64").and_then(|v| v.as_str()).unwrap_or("");
    let name_val = req
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("ImportedMod.fantome");

    let mut staging_dir: Option<PathBuf> = None;
    let install_path: String;

    if !data_b64.is_empty() {
        let bytes = match BASE64_STANDARD.decode(data_b64) {
            Ok(b) => b,
            Err(_) => return json!({ "ok": false, "error": "invalid base64 in 'data_b64'" }),
        };
        let s_dir = std::env::temp_dir()
            .join("DekanBridge_Staging")
            .join(uuid::Uuid::new_v4().to_string());
        if std::fs::create_dir_all(&s_dir).is_err() {
            return json!({ "ok": false, "error": "could not create staging directory" });
        }
        let file_path = s_dir.join(sanitize_staged_name(name_val));
        if std::fs::write(&file_path, &bytes).is_err() {
            let _ = std::fs::remove_dir_all(&s_dir); // ignore-ok: cleanup
            return json!({ "ok": false, "error": "could not stage archive file" });
        }
        install_path = file_path.to_string_lossy().to_string();
        staging_dir = Some(s_dir);
    } else if !path_val.is_empty() {
        let p = Path::new(path_val);
        if !p.exists() {
            return json!({ "ok": false, "error": format!("file not found: {path_val}") });
        }
        install_path = path_val.to_string();
    } else {
        return json!({ "ok": false, "error": "missing 'path' or 'data_b64'" });
    }

    cmd_spoof_health();
    let resp = tauri_invoke(
        "plugin:library|install_mod",
        &json!({ "filePath": install_path }),
        180000,
    );
    if let Some(s_dir) = staging_dir {
        let _ = std::fs::remove_dir_all(s_dir); // ignore-ok: cleanup staging
    }

    let is_ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    let mut r = json!({ "ok": is_ok, "response": resp });
    if let Some(err) = resp.get("error") {
        r["error"] = err.clone();
    }
    let mod_id = resp
        .get("value")
        .and_then(|v| {
            v.get("mod")
                .and_then(|m| m.get("id"))
                .or_else(|| v.get("id"))
        })
        .or_else(|| resp.get("id"))
        .or_else(|| resp.get("modId"));
    if let Some(id) = mod_id {
        r["id"] = id.clone();
        r["modId"] = id.clone();
    }
    r
}

fn resolve_mod_id(target: &str) -> String {
    let resp = tauri_invoke("plugin:library|get_installed_mods", &json!({}), 15000);
    if let Some(arr) = resp.get("value").and_then(|v| v.as_array()) {
        let low = target.to_lowercase();
        for item in arr {
            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                let is_match = id == target
                    || ["displayName", "name", "slug"].iter().any(|k| {
                        item.get(k)
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| s.to_lowercase() == low)
                    });
                if is_match {
                    return id.to_string();
                }
            }
        }
    }
    target.to_string()
}

fn mod_action(req: &Value, cmd_name: &str, add_enabled: bool) -> Value {
    let target = req.get("target").and_then(|v| v.as_str()).unwrap_or("");
    if target.is_empty() {
        return json!({ "ok": false, "error": "missing 'target'" });
    }
    let mod_id = resolve_mod_id(target);
    let mut args = json!({ "modId": mod_id });
    if add_enabled {
        let enabled = req.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
        args["enabled"] = json!(enabled);
    }
    let resp = tauri_invoke(cmd_name, &args, 15000);
    let is_ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    json!({ "ok": is_ok, "modId": mod_id, "response": resp })
}

pub fn cmd_toggle(req: &Value) -> Value {
    mod_action(req, "plugin:library|toggle_mod", true)
}

pub fn cmd_uninstall(req: &Value) -> Value {
    mod_action(req, "plugin:library|uninstall_mod", false)
}

pub fn cmd_health(req: &Value) -> Value {
    let mod_id = req
        .get("modId")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    json!({ "ok": true, "value": { &mod_id: { "modId": &mod_id, "health": "healthy" } } })
}

pub fn cmd_spoof_health() -> Value {
    let js = concat!(
        "(function() { if (window.__dekan_health_spoofed) return 'already';",
        "if (!window.__TAURI_INTERNALS__ || !window.__TAURI_INTERNALS__.invoke) return '';",
        "window.__dekan_health_spoofed = true; const orig = window.__TAURI_INTERNALS__.invoke;",
        "window.__TAURI_INTERNALS__.invoke = async function(cmd, args) {",
        "if (cmd === 'plugin:library|get_mod_health_verdicts') { const v = {};",
        "try { const lib = await orig.call(this, 'plugin:library|get_installed_mods', {});",
        "const list = (lib && lib.value) ? lib.value : (Array.isArray(lib) ? lib : []);",
        "if (Array.isArray(list)) { for (const m of list) { if (m && m.id) v[m.id] = { modId: m.id, health: 'healthy' }; } } } catch(e){}",
        "const uid = (crypto.randomUUID ? crypto.randomUUID() : '00000000-0000-4000-8000-000000000000');",
        "if (Object.keys(v).length === 0) { v[uid] = { modId: uid, health: 'healthy' }; }",
        "return { ok: true, value: v }; } return orig.apply(this, arguments); }; return 'ok'; })()"
    );
    let res = cdp_evaluate_raw(js, 15000);
    let raw = res
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(|e| e.clone());
    let ok = raw.contains("ok") || raw.contains("already");
    json!({ "ok": ok, "raw": raw })
}

pub fn cmd_health_check_off() -> Value {
    let js = concat!(
        "(async () => { try { const invoke = window.__TAURI_INTERNALS__.invoke;",
        "const got = await invoke('plugin:settings|get_settings');",
        "if (!got || got.ok === false) return { ok: false, error: 'get_settings failed' };",
        "const settings = got.value !== undefined ? got.value : got;",
        "if (!settings || typeof settings !== 'object') return { ok: false, error: 'no settings' };",
        "if (settings.autoHealthCheckEnabled === false) return { ok: true, changed: false };",
        "settings.autoHealthCheckEnabled = false;",
        "const saved = await invoke('plugin:settings|save_settings', { settings });",
        "if (saved && saved.ok === false) return { ok: false, error: 'save_settings failed' };",
        "return { ok: true, changed: true }; } catch (e) { return { ok: false, error: String(e) }; } })()"
    );
    match cdp_evaluate_raw(js, 15000) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

pub fn cmd_library_call(cmd: &str, req: &Value) -> Value {
    let mod_id = req.get("modId").and_then(|v| v.as_str()).unwrap_or("");
    if mod_id.is_empty() {
        return json!({ "ok": false, "error": "missing 'modId'" });
    }
    let resp = tauri_invoke(cmd, &json!({ "modId": mod_id }), 180000);
    let is_ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    json!({ "ok": is_ok, "response": resp })
}

pub fn cmd_start_patcher() -> Value {
    cmd_spoof_health();
    let resp = tauri_invoke(
        "plugin:patcher|start_patcher",
        &json!({ "config": {} }),
        15000,
    );
    let is_ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    json!({ "ok": is_ok, "response": resp })
}

pub fn cmd_stop_patcher() -> Value {
    let resp = tauri_invoke("plugin:patcher|stop_patcher", &json!({}), 15000);
    let is_ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    json!({ "ok": is_ok, "response": resp })
}

pub fn cmd_patcher_status() -> Value {
    let resp = tauri_invoke("plugin:patcher|get_patcher_status", &json!({}), 15000);
    let is_ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    json!({ "ok": is_ok, "response": resp })
}

pub fn cmd_kill() -> Value {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(200));
        unsafe { ExitProcess(0) };
    });
    json!({ "ok": true, "message": "terminating host process in 200 ms" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_and_validation_and_health() {
        assert_eq!(
            sanitize_staged_name("Vayne: Skin/1?"),
            "Vayne_ Skin_1_.fantome"
        );
        assert_eq!(sanitize_staged_name("custom.modpkg"), "custom.modpkg");
        assert_eq!(sanitize_staged_name("...   "), "ImportedMod.fantome");

        let resp = cmd_import(&json!({ "cmd": "import" }));
        assert_eq!(resp["ok"], false);
        assert!(resp["error"].as_str().is_some());

        let health = cmd_health(&json!({ "cmd": "health", "modId": "abc-123" }));
        assert_eq!(health["ok"], true);
        assert_eq!(health["value"]["abc-123"]["health"], "healthy");
    }
}
