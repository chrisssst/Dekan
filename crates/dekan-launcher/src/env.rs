use std::net::TcpListener;
use std::path::Path;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};

pub fn pick_free_port() -> u16 {
    if let Ok(listener) = TcpListener::bind("127.0.0.1:0") {
        if let Ok(addr) = listener.local_addr() {
            return addr.port();
        }
    }
    9222
}

pub fn setup_environment(
    port: u16,
    no_hide: bool,
    log_dir: Option<&Path>,
) -> Vec<(String, String)> {
    let browser_args = format!(
        "--remote-debugging-port={} --remote-allow-origins=http://127.0.0.1:{}",
        port, port
    );
    let port_str = port.to_string();
    let mut vars = vec![
        (
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS".to_string(),
            browser_args,
        ),
        ("LTF_CDP_PORT".to_string(), port_str),
    ];
    if no_hide {
        vars.push(("DEKAN_BRIDGE_NO_HIDE".to_string(), "1".to_string()));
    }
    if let Some(dir) = log_dir {
        vars.push(("LTF_LOG_DIR".to_string(), dir.to_string_lossy().to_string()));
    }
    for (k, v) in &vars {
        unsafe {
            std::env::set_var(k, v);
        }
    }
    vars
}

pub fn environment_for_token(token: HANDLE, vars: &[(String, String)]) -> Vec<u16> {
    let mut block: Vec<u16> = Vec::new();
    let mut env_ptr: *mut core::ffi::c_void = std::ptr::null_mut();

    let created = unsafe { CreateEnvironmentBlock(&mut env_ptr, Some(token), false) };
    if created.is_ok() && !env_ptr.is_null() {
        let mut cur = env_ptr as *const u16;
        unsafe {
            while *cur != 0 {
                let mut len = 0;
                while *cur.add(len) != 0 {
                    len += 1;
                }
                let entry_slice = std::slice::from_raw_parts(cur, len);
                let entry_str = String::from_utf16_lossy(entry_slice);
                let check_str = entry_str.strip_prefix('=').unwrap_or(&entry_str);

                let key = if let Some(eq) = check_str.find('=') {
                    &check_str[..eq]
                } else {
                    ""
                };

                let replaced = vars.iter().any(|(k, _)| k.eq_ignore_ascii_case(key));
                if !replaced {
                    block.extend_from_slice(entry_slice);
                    block.push(0);
                }
                cur = cur.add(len + 1);
            }
            let _ = DestroyEnvironmentBlock(env_ptr); // ignore-ok: cleanup of user environment block
        }
    }

    for (k, v) in vars {
        let item = format!("{k}={v}");
        block.extend(item.encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

pub fn with_env_var(block: &[u16], name: &str, value: &str) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    let mut at = 0;

    while at < block.len() && block[at] != 0 {
        let mut end = at;
        while end < block.len() && block[end] != 0 {
            end += 1;
        }
        let entry_slice = &block[at..end];
        let entry_str = String::from_utf16_lossy(entry_slice);
        let check_str = entry_str.strip_prefix('=').unwrap_or(&entry_str);

        let key = if let Some(eq) = check_str.find('=') {
            &check_str[..eq]
        } else {
            ""
        };

        if !key.eq_ignore_ascii_case(name) {
            out.extend_from_slice(entry_slice);
            out.push(0);
        }
        at = end + 1;
    }

    let addition = format!("{name}={value}");
    out.extend(addition.encode_utf16());
    out.push(0);
    out.push(0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_free_port_returns_nonzero() {
        let port = pick_free_port();
        assert!(port > 0);
    }

    #[test]
    fn with_env_var_replaces_case_insensitive_and_keeps_drive_entries() {
        let mut block = Vec::new();
        for entry in ["=C:=C:\\", "PATH=C:\\Windows", "LTF_CDP_PORT=9222"] {
            block.extend(entry.encode_utf16());
            block.push(0);
        }
        block.push(0);

        let updated = with_env_var(&block, "ltf_cdp_port", "45123");
        let text = String::from_utf16_lossy(&updated);
        let parts: Vec<&str> = text.split('\0').filter(|s| !s.is_empty()).collect();
        assert_eq!(
            parts,
            vec!["=C:=C:\\", "PATH=C:\\Windows", "ltf_cdp_port=45123"]
        );
    }
}
