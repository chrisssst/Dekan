use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::Security::Cryptography::{CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
    WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvSignerFromChain,
    WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows::core::HSTRING;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    #[error("{path} has no valid Authenticode signature (WinVerifyTrust 0x{code:08x})")]
    Untrusted { path: String, code: u32 },

    #[error("{path} is signed but its signer certificate could not be read")]
    NoSigner { path: String },
}

pub fn signer(path: &Path) -> Result<String, SignatureError> {
    let wide = HSTRING::from(path.as_os_str());
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: windows::core::PCWSTR(wide.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let verdict = unsafe {
        WinVerifyTrust(
            HWND::default(),
            &mut action,
            std::ptr::addr_of_mut!(data).cast(),
        )
    };
    let name = (verdict == 0).then(|| signer_name(&data)).flatten();
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        WinVerifyTrust(
            HWND::default(),
            &mut action,
            std::ptr::addr_of_mut!(data).cast(),
        );
    }
    let shown = path.display().to_string();
    if verdict != 0 {
        return Err(SignatureError::Untrusted {
            path: shown,
            code: verdict as u32,
        });
    }
    name.ok_or(SignatureError::NoSigner { path: shown })
}

fn signer_name(data: &WINTRUST_DATA) -> Option<String> {
    unsafe {
        let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
        if provider.is_null() {
            return None;
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0).as_ref()?;
        if signer.csCertChain == 0 || signer.pasCertChain.is_null() {
            return None;
        }
        let cert = (*signer.pasCertChain).pCert;
        if cert.is_null() {
            return None;
        }
        let mut buffer = [0u16; 256];
        let written = CertGetNameStringW(
            cert,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            None,
            Some(&mut buffer),
        );
        let len = (written as usize).saturating_sub(1).min(buffer.len());
        (len > 0).then(|| String::from_utf16_lossy(&buffer[..len]))
    }
}

#[cfg(test)]
mod tests;
