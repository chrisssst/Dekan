use std::path::Path;

use dekan_platform::authenticode::SignatureError;

use crate::error::InjectError;

pub const LTK_PUBLISHER: &str = "Natoken LLC";

const TRUST_E_NOSIGNATURE: u32 = 0x800b_0100;

const PLAUSIBLE_LIMITS: std::ops::RangeInclusive<u32> = 1_735_689_600..=2_208_988_800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Signed,
    Unsigned,
}

impl Provenance {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Signed => "signed by the publisher",
            Self::Unsigned => "unsigned",
        }
    }
}

pub fn verify_injector_file(path: &Path) -> Result<Provenance, InjectError> {
    judge(dekan_platform::authenticode::signer(path)).map_err(|reason| {
        InjectError::UntrustedInjector {
            path: path.display().to_string(),
            reason,
        }
    })
}

fn judge(signature: Result<String, SignatureError>) -> Result<Provenance, String> {
    match signature {
        Ok(publisher) if publisher == LTK_PUBLISHER => Ok(Provenance::Signed),
        Ok(publisher) => Err(format!("signed by {publisher}, not by {LTK_PUBLISHER}")),
        Err(SignatureError::Untrusted {
            code: TRUST_E_NOSIGNATURE,
            ..
        }) => Ok(Provenance::Unsigned),
        Err(e) => Err(e.to_string()),
    }
}

#[must_use]
pub fn dll_refuses_other_hosts(dll: &[u8]) -> bool {
    dll.windows(HOST_LOCK_MARK.len())
        .any(|w| w == HOST_LOCK_MARK)
}

const HOST_LOCK_MARK: &[u8] = b"host verification failed";

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
mod tests;
