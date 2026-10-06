use super::*;

#[test]
fn every_user_supplied_tool_names_audited_hashes_the_app_declares() {
    for (name, constants) in USER_SUPPLIED_TOOLS {
        for constant in constants {
            let hash = audited_hash(constant).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(hash.len(), 64);
        }
    }
}

#[test]
fn the_ltk_dll_that_failed_in_a_match_is_accepted_nowhere() {
    const FAILED_2040_DLL: &str =
        "1812c9f032a2a96d4464df7ede84b4effa2b4762d75ccbffbd82101491b79357";
    assert!(!TRIGGER_SOURCE.contains(FAILED_2040_DLL));
    for (_, constants) in USER_SUPPLIED_TOOLS {
        for constant in constants {
            assert_ne!(audited_hash(constant).expect("declared"), FAILED_2040_DLL);
        }
    }
}

#[test]
fn an_undeclared_constant_is_an_error_not_a_pass() {
    assert!(audited_hash("AUDITED_DOES_NOT_EXIST_HASH").is_err());
}
