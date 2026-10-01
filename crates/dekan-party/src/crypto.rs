use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Aead, Generate, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::PartyError;

pub const BLOB_VERSION: u32 = 1;

pub const MAX_CIPHERTEXT_B64: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedBlob {
    pub v: u32,

    pub n: String,

    pub c: String,
}

#[must_use]
pub fn room_id(key: &[u8; 32]) -> String {
    let digest = Sha256::new()
        .chain_update(b"dekan-party/room/v1")
        .chain_update(key)
        .finalize();
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

pub struct RoomCipher {
    cipher: XChaCha20Poly1305,
}

impl RoomCipher {
    #[must_use]
    pub fn new(key: &[u8; 32]) -> Self {
        let derived = Sha256::new()
            .chain_update(b"dekan-party/aead/v1")
            .chain_update(key)
            .finalize();
        let key: [u8; 32] = derived.into();
        Self {
            cipher: XChaCha20Poly1305::new(&Key::from(key)),
        }
    }

    pub fn seal(&self, plaintext: &[u8]) -> Result<SealedBlob, PartyError> {
        // Fallible on purpose: an OS RNG failure refuses to send instead of panicking.
        let nonce = XNonce::try_generate()
            .map_err(|e| PartyError::Crypto(format!("no randomness for a nonce: {e}")))?;
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| PartyError::Crypto("encryption failed".into()))?;
        Ok(SealedBlob {
            v: BLOB_VERSION,
            n: STANDARD.encode(nonce),
            c: STANDARD.encode(ciphertext),
        })
    }

    pub fn open(&self, blob: &SealedBlob) -> Result<Vec<u8>, PartyError> {
        if blob.v != BLOB_VERSION {
            return Err(PartyError::Crypto(format!("blob version {}", blob.v)));
        }
        if blob.c.len() > MAX_CIPHERTEXT_B64 {
            return Err(PartyError::Crypto("ciphertext too large".into()));
        }
        let nonce = STANDARD
            .decode(&blob.n)
            .map_err(|_| PartyError::Crypto("nonce is not base64".into()))?;
        let nonce = XNonce::try_from(nonce.as_slice())
            .map_err(|_| PartyError::Crypto("nonce has the wrong length".into()))?;
        let ciphertext = STANDARD
            .decode(&blob.c)
            .map_err(|_| PartyError::Crypto("ciphertext is not base64".into()))?;
        self.cipher
            .decrypt(&nonce, ciphertext.as_ref())
            .map_err(|_| PartyError::Crypto("authentication failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seal_open_round_trip_with_distinct_nonces() {
        let cipher = RoomCipher::new(&[7u8; 32]);
        let a = cipher.seal(b"hello").expect("seal");
        let b = cipher.seal(b"hello").expect("seal");
        assert_ne!(a.n, b.n, "every message gets a fresh nonce");
        assert_ne!(a.c, b.c);
        assert_eq!(cipher.open(&a).expect("open"), b"hello");
    }

    #[test]
    fn test_other_rooms_and_tampering_are_refused() {
        let ours = RoomCipher::new(&[7u8; 32]);
        let theirs = RoomCipher::new(&[8u8; 32]);
        let blob = ours.seal(b"secret").expect("seal");
        assert!(
            theirs.open(&blob).is_err(),
            "another room's key cannot read it"
        );

        let mut tampered = blob.clone();
        let mut raw = STANDARD.decode(&tampered.c).expect("b64");
        raw[0] ^= 1;
        tampered.c = STANDARD.encode(raw);
        assert!(
            ours.open(&tampered).is_err(),
            "a flipped bit fails authentication"
        );

        let oversized = SealedBlob {
            c: "A".repeat(MAX_CIPHERTEXT_B64 + 4),
            ..blob
        };
        assert!(ours.open(&oversized).is_err());
    }

    #[test]
    fn test_room_name_is_stable_and_not_the_key() {
        let key = [9u8; 32];
        assert_eq!(room_id(&key), room_id(&key));
        assert_eq!(room_id(&key).len(), 32);
        assert_ne!(room_id(&key), room_id(&[1u8; 32]));
    }
}
