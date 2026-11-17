use crate::error::PlatformError;

pub fn seal(plain: &[u8], _purpose: &str) -> Result<Vec<u8>, PlatformError> {
    Ok(plain.to_vec())
}

pub fn unseal(sealed: &[u8], _purpose: &str) -> Result<Vec<u8>, PlatformError> {
    Ok(sealed.to_vec())
}
