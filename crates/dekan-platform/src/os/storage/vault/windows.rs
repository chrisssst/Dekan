use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::PCWSTR;

use crate::error::PlatformError;

fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB, PlatformError> {
    let len = u32::try_from(bytes.len())
        .map_err(|_| PlatformError::Security("the data to protect is too large".into()))?;
    Ok(CRYPT_INTEGER_BLOB {
        cbData: len,
        pbData: bytes.as_ptr().cast_mut(),
    })
}

fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(out.pbData.cast()))) };
    bytes
}

pub fn seal(plain: &[u8], purpose: &str) -> Result<Vec<u8>, PlatformError> {
    let input = blob(plain)?;
    let entropy = blob(purpose.as_bytes())?;
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    }
    .map_err(|e| PlatformError::Security(format!("DPAPI could not protect the data: {e}")))?;
    Ok(take(out))
}

pub fn unseal(sealed: &[u8], purpose: &str) -> Result<Vec<u8>, PlatformError> {
    let input = blob(sealed)?;
    let entropy = blob(purpose.as_bytes())?;
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    }
    .map_err(|e| PlatformError::Security(format!("DPAPI could not open the data: {e}")))?;
    Ok(take(out))
}
