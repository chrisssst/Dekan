use super::{seal, unseal};

#[test]
fn sealed_data_opens_with_the_same_purpose() {
    let sealed = seal(b"grant", "dekan-test").expect("seal");
    assert_eq!(unseal(&sealed, "dekan-test").expect("unseal"), b"grant");
}

#[cfg(windows)]
#[test]
fn sealed_data_is_not_plain_text() {
    let sealed = seal(b"grant-in-the-clear", "dekan-test").expect("seal");
    assert!(!sealed.windows(5).any(|w| w == b"grant"));
}

#[cfg(windows)]
#[test]
fn another_purpose_cannot_open_the_data() {
    let sealed = seal(b"grant", "dekan-test").expect("seal");
    assert!(unseal(&sealed, "other-purpose").is_err());
}

#[cfg(windows)]
#[test]
fn tampered_data_is_rejected() {
    let mut sealed = seal(b"grant", "dekan-test").expect("seal");
    let last = sealed.len() - 1;
    sealed[last] ^= 0xff;
    assert!(unseal(&sealed, "dekan-test").is_err());
}
