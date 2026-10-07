use std::collections::HashSet;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dekan_party::crypto::{MAX_CIPHERTEXT_B64, RoomCipher, SealedBlob, room_id};
use dekan_party::token::{PartyToken, TOKEN_PREFIX, TOKEN_TTL_SECS};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn key(&mut self) -> [u8; 32] {
        std::array::from_fn(|_| self.next() as u8)
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn with_bytes(blob: &SealedBlob, nonce: &[u8], ciphertext: &[u8]) -> SealedBlob {
    SealedBlob {
        v: blob.v,
        n: STANDARD.encode(nonce),
        c: STANDARD.encode(ciphertext),
    }
}

#[test]
fn every_single_bit_flip_in_nonce_or_ciphertext_is_refused() {
    let mut rng = Rng(0x5EED_0101);
    let cipher = RoomCipher::new(&rng.key());
    let plaintext = rng.bytes(96);
    let blob = cipher.seal(&plaintext).expect("seal");
    let nonce = STANDARD.decode(&blob.n).expect("nonce");
    let ciphertext = STANDARD.decode(&blob.c).expect("ciphertext");
    assert_eq!(cipher.open(&blob).expect("open"), plaintext);

    for bit in 0..nonce.len() * 8 {
        let mut flipped = nonce.clone();
        flipped[bit / 8] ^= 1 << (bit % 8);
        assert!(
            cipher
                .open(&with_bytes(&blob, &flipped, &ciphertext))
                .is_err(),
            "nonce bit {bit}"
        );
    }
    for bit in 0..ciphertext.len() * 8 {
        let mut flipped = ciphertext.clone();
        flipped[bit / 8] ^= 1 << (bit % 8);
        assert!(
            cipher.open(&with_bytes(&blob, &nonce, &flipped)).is_err(),
            "ciphertext bit {bit}"
        );
    }
    for cut in 0..ciphertext.len() {
        assert!(
            cipher
                .open(&with_bytes(&blob, &nonce, &ciphertext[..cut]))
                .is_err(),
            "cut {cut}"
        );
    }
}

#[test]
fn random_payloads_round_trip_and_nonces_never_repeat() {
    let mut rng = Rng(0x5EED_0102);
    let cipher = RoomCipher::new(&rng.key());
    let mut nonces = HashSet::new();
    for round in 0..20_000 {
        let len = (rng.next() % 1500) as usize;
        let plaintext = rng.bytes(len);
        let blob = cipher.seal(&plaintext).expect("seal");
        assert!(nonces.insert(blob.n.clone()), "nonce repeated at {round}");
        if round % 20 == 0 {
            assert_eq!(cipher.open(&blob).expect("open"), plaintext);
            assert_ne!(
                STANDARD.decode(&blob.c).expect("c")[..plaintext.len().min(16)],
                plaintext[..plaintext.len().min(16)]
            );
        }
    }
}

#[test]
fn a_blob_never_opens_in_another_room_and_rooms_do_not_reveal_keys() {
    let mut rng = Rng(0x5EED_0103);
    let key = rng.key();
    let blob = RoomCipher::new(&key)
        .seal(b"skin 238001 chroma 7")
        .expect("seal");
    let mut rooms = HashSet::new();
    for _ in 0..500 {
        let other = rng.key();
        assert!(RoomCipher::new(&other).open(&blob).is_err());
        let room = room_id(&other);
        assert_eq!(room.len(), 32);
        assert!(
            !room.contains(
                &other
                    .iter()
                    .take(4)
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            )
        );
        assert!(rooms.insert(room), "room ids collide");
    }
    assert_eq!(room_id(&key), room_id(&key));
}

#[test]
fn malformed_blobs_are_refused_before_any_decryption_work() {
    let cipher = RoomCipher::new(&[7; 32]);
    let blob = cipher.seal(b"payload").expect("seal");
    for bad in [
        SealedBlob {
            v: 2,
            ..blob.clone()
        },
        SealedBlob {
            c: "A".repeat(MAX_CIPHERTEXT_B64 + 4),
            ..blob.clone()
        },
        SealedBlob {
            n: "not base64!".into(),
            ..blob.clone()
        },
        SealedBlob {
            n: STANDARD.encode([0u8; 12]),
            ..blob.clone()
        },
        SealedBlob {
            c: "%%%".into(),
            ..blob.clone()
        },
    ] {
        assert!(cipher.open(&bad).is_err(), "{bad:?}");
    }
}

#[test]
fn invite_codes_expire_exactly_at_their_limits_and_mutations_never_panic() {
    let now = 1_800_000_000;
    let token = PartyToken::generate(42, now).expect("token");
    let code = token.encode();
    assert!(code.starts_with(TOKEN_PREFIX));
    assert_eq!(PartyToken::decode(&code, now).expect("fresh"), token);
    assert!(PartyToken::decode(&code, now + TOKEN_TTL_SECS).is_ok());
    assert!(PartyToken::decode(&code, now + TOKEN_TTL_SECS + 1).is_err());
    assert!(
        PartyToken::decode(&code, now - 300).is_ok(),
        "five minutes of clock skew are tolerated"
    );
    assert!(PartyToken::decode(&code, now - 301).is_err());

    let mut rng = Rng(0x5EED_0104);
    let alphabet: Vec<char> = "ABCxyz019-_=+/:! ".chars().collect();
    for _ in 0..20_000 {
        let mut chars: Vec<char> = code.chars().collect();
        let at = (rng.next() as usize) % chars.len();
        match rng.next() % 3 {
            0 => chars[at] = alphabet[(rng.next() as usize) % alphabet.len()],
            1 => {
                chars.remove(at);
            }
            _ => chars.truncate(at),
        }
        let mutated: String = chars.into_iter().collect();
        if let Ok(decoded) = PartyToken::decode(&mutated, now) {
            assert!(
                mutated == code || decoded != token,
                "a changed code decoded to the same room: {mutated}"
            );
        }
    }
}
