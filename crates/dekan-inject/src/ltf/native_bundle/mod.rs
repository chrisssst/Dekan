use std::path::Path;

use sha2::{Digest, Sha256};

pub const LAUNCHER: &str = "dekan_launcher.exe";
pub const BRIDGE: &str = "dekan_bridge.dll";
pub const COMPAT_BRIDGE: &str = "bullet_bridge.dll";
pub const SHIPPED_TOOLS: [&str; 3] = [LAUNCHER, BRIDGE, COMPAT_BRIDGE];

const LAUNCHER_HASH: &str = "23c941d2df0041e0a8bfb7d2fb0f4504ad215fc5a8d5ec64fd95a067ac9966f5";
const BRIDGE_HASH: &str = "2556c5e8c5d834eab710b77fb8e61844ef5665f593f71b7bd311af9f9a20d7c2";

pub fn source_name(name: &str) -> Option<&'static str> {
    match name {
        LAUNCHER => Some(LAUNCHER),
        BRIDGE | COMPAT_BRIDGE => Some(BRIDGE),
        _ => None,
    }
}

pub fn verify_bytes(name: &str, bytes: &[u8]) -> Result<(), String> {
    let expected = match name {
        LAUNCHER => LAUNCHER_HASH,
        BRIDGE | COMPAT_BRIDGE => BRIDGE_HASH,
        _ => return Err(format!("Unknown Dekan native component: {name}")),
    };
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err(format!(
            "Dekan native component {name} does not match this release; reinstall Dekan 2.2"
        ));
    }
    Ok(())
}

pub fn verify_file(path: &Path) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Invalid Dekan native component path".to_owned())?;
    let bytes = std::fs::read(path).map_err(|e| {
        format!(
            "Could not read Dekan native component {}: {e}",
            path.display()
        )
    })?;
    verify_bytes(name, &bytes)
}

pub fn verify_directory(dir: &Path) -> Result<(), String> {
    for name in SHIPPED_TOOLS {
        verify_file(&dir.join(name))?;
    }
    Ok(())
}

pub fn copy_bundle(source: &Path, destination: &Path) -> Result<(), String> {
    for name in [LAUNCHER, BRIDGE] {
        verify_file(&source.join(name))?;
    }
    std::fs::create_dir_all(destination)
        .map_err(|e| format!("Could not create native tools folder: {e}"))?;
    for name in SHIPPED_TOOLS {
        let input = source_name(name).ok_or_else(|| "Invalid native tool manifest".to_owned())?;
        std::fs::copy(source.join(input), destination.join(name))
            .map_err(|e| format!("Could not copy Dekan native component {name}: {e}"))?;
    }
    verify_directory(destination)
}

#[cfg(test)]
mod tests;
