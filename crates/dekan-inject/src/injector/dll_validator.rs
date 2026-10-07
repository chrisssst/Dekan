use sha2::{Digest, Sha256};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sha256_known_vector() {
        let data = b"hello world";

        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert_eq!(compute_sha256(data), expected);
    }
}
