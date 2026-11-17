use std::path::Path;

use dekan_platform::authenticode::SignatureError;
use tracing::warn;

use crate::error::InjectError;

pub const NATIVE_PUBLISHER: Option<&str> = option_env!("DEKAN_NATIVE_PUBLISHER");

pub fn verify_native_file(path: &Path) -> Result<(), InjectError> {
    let Some(expected) = NATIVE_PUBLISHER.filter(|p| !p.is_empty()) else {
        warn!(path = %path.display(), "This build pins no publisher for the native engine; its signature is not checked");
        return Ok(());
    };
    judge_native(expected, dekan_platform::authenticode::signer(path)).map_err(|reason| {
        InjectError::UntrustedInjector {
            path: path.display().to_string(),
            reason,
        }
    })
}

fn judge_native(expected: &str, signature: Result<String, SignatureError>) -> Result<(), String> {
    match signature {
        Ok(publisher) if publisher == expected => Ok(()),
        Ok(publisher) => Err(format!("signed by {publisher}, not by {expected}")),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests;
