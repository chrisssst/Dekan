use std::path::Path;

use crate::error::InjectError;

pub const LTK_PUBLISHER: &str = "Natoken LLC";

const PLAUSIBLE_LIMITS: std::ops::RangeInclusive<u32> = 1_735_689_600..=2_208_988_800;

pub fn verify_injector_file(path: &Path) -> Result<(), InjectError> {
    match dekan_platform::authenticode::signer(path) {
        Ok(publisher) if publisher == LTK_PUBLISHER => Ok(()),
        Ok(publisher) => Err(InjectError::UntrustedInjector {
            path: path.display().to_string(),
            reason: format!("signed by {publisher}, not by {LTK_PUBLISHER}"),
        }),
        Err(e) => Err(InjectError::UntrustedInjector {
            path: path.display().to_string(),
            reason: e.to_string(),
        }),
    }
}

#[must_use]
pub fn dll_build_limit(dll: &[u8]) -> Option<u32> {
    let mut found = dll.windows(7).filter_map(|code| match code {
        [0x3d, a, b, c, d, 0x0f, 0x86] => {
            let limit = u32::from_le_bytes([*a, *b, *c, *d]);
            (PLAUSIBLE_LIMITS.contains(&limit) && limit % 3600 == 0).then_some(limit)
        }
        _ => None,
    });
    let first = found.next()?;
    found.all(|other| other == first).then_some(first)
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
