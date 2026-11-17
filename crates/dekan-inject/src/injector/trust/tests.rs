use super::*;

const TRUST_E_NOSIGNATURE: u32 = 0x800b_0100;

#[test]
fn the_native_engine_must_carry_the_pinned_publisher() {
    assert_eq!(judge_native("Dekan", Ok("Dekan".into())), Ok(()));
    assert!(
        judge_native("Dekan", Ok("Someone Else".into()))
            .is_err_and(|reason| reason.contains("Someone Else"))
    );
    let unsigned = SignatureError::Untrusted {
        path: "dekan-br.dll".into(),
        code: TRUST_E_NOSIGNATURE,
    };
    assert!(judge_native("Dekan", Err(unsigned)).is_err());
}
