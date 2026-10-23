use std::path::Path;
use std::process::Command;

use super::SignatureError;

const CODESIGN: &str = "/usr/bin/codesign";

pub fn signer(path: &Path) -> Result<String, SignatureError> {
    let shown = path.display().to_string();
    let untrusted = |code: i32| SignatureError::Untrusted {
        path: shown.clone(),
        code: code as u32,
    };
    let verified = Command::new(CODESIGN)
        .args(["--verify", "--strict"])
        .arg(path)
        .output()
        .map_err(|_| untrusted(-1))?;
    if !verified.status.success() {
        return Err(untrusted(verified.status.code().unwrap_or(-1)));
    }
    let details = Command::new(CODESIGN)
        .args(["-dv", "--verbose=2"])
        .arg(path)
        .output()
        .map_err(|_| untrusted(-1))?;
    leaf_authority(&String::from_utf8_lossy(&details.stderr))
        .ok_or(SignatureError::NoSigner { path: shown })
}

fn leaf_authority(details: &str) -> Option<String> {
    details
        .lines()
        .find_map(|line| line.strip_prefix("Authority="))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_leaf_authority_is_the_first_one_listed() {
        let details = "Executable=/x\nAuthority=Developer ID Application: Someone (TEAM1)\nAuthority=Developer ID Certification Authority\nAuthority=Apple Root CA\n";
        assert_eq!(
            leaf_authority(details).as_deref(),
            Some("Developer ID Application: Someone (TEAM1)")
        );
        assert_eq!(leaf_authority("Signature=adhoc\n"), None);
    }

    #[test]
    fn an_unsigned_file_is_untrusted() {
        let path = std::env::temp_dir().join(format!("dekan_sig_{}", std::process::id()));
        std::fs::write(&path, b"not a program").expect("fixture");
        assert!(matches!(
            signer(&path),
            Err(SignatureError::Untrusted { .. })
        ));
        std::fs::remove_file(path).ok();
    }
}
