#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::signer;
#[cfg(windows)]
pub use windows::signer;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    #[cfg_attr(
        windows,
        error("{path} has no valid Authenticode signature (WinVerifyTrust 0x{code:08x})")
    )]
    #[cfg_attr(
        target_os = "macos",
        error("{path} has no valid code signature (codesign status {code})")
    )]
    Untrusted { path: String, code: u32 },

    #[error("{path} is signed but its signer certificate could not be read")]
    NoSigner { path: String },
}

#[cfg(all(test, windows))]
mod tests;
