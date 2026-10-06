use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::Generate;

use crate::error::PartyError;

fn no_randomness(e: impl std::fmt::Display) -> PartyError {
    PartyError::Crypto(format!("the system random generator failed: {e}"))
}

pub const TOKEN_PREFIX: &str = "DEKAN1:";
const TOKEN_VERSION: u8 = 1;
const TOKEN_LEN: usize = 1 + 8 + 8 + 32;

pub const TOKEN_TTL_SECS: u64 = 3600;

const CLOCK_SKEW_SECS: u64 = 300;

#[derive(Clone, PartialEq, Eq)]
pub struct PartyToken {
    pub issued_at: u64,

    pub host_member: u64,
    key: [u8; 32],
}

impl std::fmt::Debug for PartyToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartyToken")
            .field("issued_at", &self.issued_at)
            .field("host_member", &self.host_member)
            .field("key", &"<redacted>")
            .finish()
    }
}

pub fn random_member_id() -> Result<u64, PartyError> {
    loop {
        let id = u64::try_generate().map_err(no_randomness)? & ((1u64 << 53) - 1);
        if id != 0 {
            return Ok(id);
        }
    }
}

impl PartyToken {
    pub fn generate(host_member: u64, now: u64) -> Result<Self, PartyError> {
        Ok(Self {
            issued_at: now,
            host_member,
            key: <[u8; 32]>::try_generate().map_err(no_randomness)?,
        })
    }

    #[must_use]
    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    #[must_use]
    pub fn encode(&self) -> String {
        let mut raw = Vec::with_capacity(TOKEN_LEN);
        raw.push(TOKEN_VERSION);
        raw.extend_from_slice(&self.issued_at.to_be_bytes());
        raw.extend_from_slice(&self.host_member.to_be_bytes());
        raw.extend_from_slice(&self.key);
        format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(raw))
    }

    pub fn decode(text: &str, now: u64) -> Result<Self, PartyError> {
        let text = text.trim();
        let body = text.strip_prefix(TOKEN_PREFIX).ok_or_else(|| {
            PartyError::InvalidToken(format!("it does not start with {TOKEN_PREFIX}"))
        })?;
        let raw = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| PartyError::InvalidToken("not valid base64url".into()))?;
        if raw.len() != TOKEN_LEN {
            return Err(PartyError::InvalidToken(format!(
                "{} bytes instead of {TOKEN_LEN}",
                raw.len()
            )));
        }
        if raw[0] != TOKEN_VERSION {
            return Err(PartyError::InvalidToken(format!(
                "version {} is not supported",
                raw[0]
            )));
        }
        let field = |range: std::ops::Range<usize>| -> Result<[u8; 8], PartyError> {
            raw[range]
                .try_into()
                .map_err(|_| PartyError::InvalidToken("truncated field".into()))
        };
        let issued_at = u64::from_be_bytes(field(1..9)?);
        let host_member = u64::from_be_bytes(field(9..17)?);
        let mut key = [0u8; 32];
        key.copy_from_slice(&raw[17..49]);

        if issued_at > now.saturating_add(CLOCK_SKEW_SECS) {
            return Err(PartyError::InvalidToken(
                "issued in the future; check the system clock".into(),
            ));
        }
        let age = now.saturating_sub(issued_at);
        if age > TOKEN_TTL_SECS {
            return Err(PartyError::ExpiredToken(age - TOKEN_TTL_SECS));
        }
        Ok(Self {
            issued_at,
            host_member,
            key,
        })
    }
}

#[must_use]
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_trip_and_expiry() {
        let token = PartyToken::generate(42, 1_000_000).expect("rng");
        let code = token.encode();
        assert!(code.starts_with(TOKEN_PREFIX));
        assert_eq!(PartyToken::decode(&code, 1_000_010).expect("fresh"), token);
        assert_eq!(
            PartyToken::decode(&format!("  {code}\n"), 1_000_010).expect("pasted with spaces"),
            token
        );
        assert!(matches!(
            PartyToken::decode(&code, 1_000_000 + TOKEN_TTL_SECS + 5),
            Err(PartyError::ExpiredToken(5))
        ));
        assert!(PartyToken::decode(&code, 1_000_000 - CLOCK_SKEW_SECS - 1).is_err());
    }

    #[test]
    fn test_rejects_foreign_or_damaged_codes() {
        let code = PartyToken::generate(1, 100).expect("rng").encode();
        assert!(
            PartyToken::decode("OTHER:abcdef", 100).is_err(),
            "codes from other clients are not ours"
        );
        assert!(PartyToken::decode(&code[..code.len() - 4], 100).is_err());
        assert!(PartyToken::decode("DEKAN1:!!!", 100).is_err());
    }

    #[test]
    fn test_debug_never_shows_the_key() {
        let token = PartyToken::generate(1, 100).expect("rng");
        let printed = format!("{token:?}");
        assert!(printed.contains("<redacted>"));
        assert!(!printed.contains(&format!("{:?}", token.key())));
    }

    #[test]
    fn test_member_ids_fit_a_javascript_number() {
        for _ in 0..1000 {
            let id = random_member_id().expect("rng");
            assert!((1..(1u64 << 53)).contains(&id));
        }
    }
}
