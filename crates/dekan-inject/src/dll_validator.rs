use std::path::Path;

use sha2::{Digest, Sha256};

use tracing::{debug, error, warn};

use crate::error::InjectError;

#[must_use]
pub fn compute_sha256(data: &[u8]) -> String {
    to_hex(&Sha256::digest(data))
}

#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}"); // ignore-ok: writing to a String cannot fail
            out
        })
}

pub fn validate_binary_hashes(path: &Path, expected_hashes: &[&str]) -> Result<(), InjectError> {
    if !path.exists() {
        warn!(path = %path.display(), "Injection binary not found on disk");
        return Err(InjectError::Process(format!(
            "Binary file not found: {}",
            path.display()
        )));
    }

    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Injection binary could not be read");
            return Err(InjectError::Io(e));
        }
    };
    let actual_hash = compute_sha256(&bytes);

    if !expected_hashes
        .iter()
        .any(|h| actual_hash.eq_ignore_ascii_case(h))
    {
        error!(
            path = %path.display(),
            bytes = bytes.len(),
            actual = %actual_hash,
            expected = ?expected_hashes,
            "Binary hash does not match any of the audited samples"
        );
        return Err(InjectError::DllHashMismatch {
            expected: expected_hashes
                .first()
                .copied()
                .unwrap_or("")
                .to_lowercase(),
            actual: actual_hash,
        });
    }

    debug!(
        path = %path.display(),
        bytes = bytes.len(),
        hash = %actual_hash,
        "Binary hash validated against the audited sample"
    );
    Ok(())
}

pub fn validate_dll_hash(path: &Path, expected_hash: &str) -> Result<(), InjectError> {
    validate_binary_hashes(path, &[expected_hash])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sha256_known_vector() {
        let data = b"hello world";

        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert_eq!(compute_sha256(data), expected);
    }

    #[test]
    fn test_validate_dll_hash_match_and_mismatch() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_sample.dll");
        let content = b"MZ fake PE file for hash test";
        std::fs::write(&test_file, content).unwrap();

        let correct_hash = compute_sha256(content);
        assert!(validate_dll_hash(&test_file, &correct_hash).is_ok());

        let result = validate_dll_hash(
            &test_file,
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert!(matches!(result, Err(InjectError::DllHashMismatch { .. })));

        let _ = std::fs::remove_file(&test_file); // ignore-ok: fixture cleanup
    }

    #[test]
    fn test_validate_binary_hashes_accepts_any_listed_sample() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join(format!("test_multi_sample_{}.dll", std::process::id()));
        let content = b"sample DLL content";
        std::fs::write(&test_file, content).unwrap();

        let correct_hash = compute_sha256(content);
        let decoy_hash = "1111111111111111111111111111111111111111111111111111111111111111";

        assert!(validate_binary_hashes(&test_file, &[decoy_hash, &correct_hash]).is_ok());
        assert!(validate_binary_hashes(&test_file, &[&correct_hash, decoy_hash]).is_ok());

        let err = validate_binary_hashes(
            &test_file,
            &[
                decoy_hash,
                "2222222222222222222222222222222222222222222222222222222222222222",
            ],
        );
        assert!(matches!(err, Err(InjectError::DllHashMismatch { .. })));

        let _ = std::fs::remove_file(test_file); // ignore-ok: fixture cleanup
    }
}
