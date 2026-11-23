use std::path::Path;

use sha2::{Digest, Sha256};

pub const LAUNCHER: &str = "dekan_launcher.exe";
pub const BRIDGE: &str = "dekan_bridge.dll";
pub const COMPAT_BRIDGE: &str = "bullet_bridge.dll";
pub const COMPAT_LAUNCHER: &str = "bullet_launcher.exe";
pub const SHIPPED_TOOLS: [&str; 4] = [LAUNCHER, BRIDGE, COMPAT_BRIDGE, COMPAT_LAUNCHER];

const LAUNCHER_HASH: &str = "7cfd70afaeeaa00cb2a0b6f900cf9c916b2b14a8ed047615b6687c7129fdf994";
const BRIDGE_HASH: &str = "73c81ad10b5e5a7d4fb8503c6436cf45d7b0af3fd09932a96ef4e22f11f89a9c";

pub fn source_name(name: &str) -> Option<&'static str> {
    match name {
        LAUNCHER | COMPAT_LAUNCHER => Some(LAUNCHER),
        BRIDGE | COMPAT_BRIDGE => Some(BRIDGE),
        _ => None,
    }
}

pub fn verify_bytes(name: &str, bytes: &[u8]) -> Result<(), String> {
    let expected = match name {
        LAUNCHER | COMPAT_LAUNCHER => LAUNCHER_HASH,
        BRIDGE | COMPAT_BRIDGE => BRIDGE_HASH,
        _ => return Err(format!("Unknown Dekan native component: {name}")),
    };
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        return Err(format!(
            "Dekan native component {name} does not match this release; reinstall Dekan 2.3"
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
