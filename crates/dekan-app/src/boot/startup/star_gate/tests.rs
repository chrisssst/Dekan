use super::github::{
    GithubError, Poll, Star, classify_star, parse_device_code, parse_poll, trusted_verification_uri,
};
use super::grant::{self, CLOCK_SKEW_SECS, Freshness, GRACE_SECS, Grant};

fn grant_at(verified_at: u64) -> Grant {
    Grant {
        login: "octocat".into(),
        user_id: 1,
        token: "gho_example".into(),
        verified_at,
    }
}

fn temp_state(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dekan-star-gate-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).expect("temp state dir");
    dir
}

#[test]
fn device_code_answer_is_read_and_github_errors_are_reported() {
    let body = r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","expires_in":900,"interval":5}"#;
    let code = parse_device_code(200, body).expect("device code");
    assert_eq!(code.user_code, "ABCD-1234");
    assert_eq!(code.interval, 5);

    let disabled = parse_device_code(
        200,
        r#"{"error":"device_flow_disabled","error_description":"Device Flow must be enabled"}"#,
    );
    assert!(
        matches!(disabled, Err(GithubError::Protocol(m)) if m.contains("device_flow_disabled"))
    );
    assert!(matches!(
        parse_device_code(500, ""),
        Err(GithubError::Status(500))
    ));
}

#[test]
fn every_device_flow_poll_answer_maps_to_its_outcome() {
    assert_eq!(
        parse_poll(
            200,
            r#"{"access_token":"gho_x","token_type":"bearer","scope":""}"#,
            5
        )
        .ok(),
        Some(Poll::Token("gho_x".into()))
    );
    assert_eq!(
        parse_poll(200, r#"{"error":"authorization_pending"}"#, 5).ok(),
        Some(Poll::Pending)
    );
    assert_eq!(
        parse_poll(200, r#"{"error":"slow_down","interval":10}"#, 5).ok(),
        Some(Poll::SlowDown(10))
    );
    assert_eq!(
        parse_poll(200, r#"{"error":"slow_down"}"#, 5).ok(),
        Some(Poll::SlowDown(10))
    );
    assert_eq!(
        parse_poll(200, r#"{"error":"expired_token"}"#, 5).ok(),
        Some(Poll::Expired)
    );
    assert_eq!(
        parse_poll(200, r#"{"error":"access_denied"}"#, 5).ok(),
        Some(Poll::Denied)
    );
    assert!(matches!(
        parse_poll(200, r#"{"error":"incorrect_device_code"}"#, 5),
        Err(GithubError::Protocol(_))
    ));
    assert!(matches!(
        parse_poll(200, "{}", 5),
        Err(GithubError::Protocol(_))
    ));
}

#[test]
fn star_answers_distinguish_missing_revoked_and_rate_limit() {
    assert_eq!(classify_star(204, None).ok(), Some(Star::Given));
    assert_eq!(classify_star(404, None).ok(), Some(Star::Missing));
    assert_eq!(classify_star(401, None).ok(), Some(Star::Revoked));
    assert!(matches!(
        classify_star(403, Some("0")),
        Err(GithubError::RateLimited)
    ));
    assert!(matches!(
        classify_star(429, None),
        Err(GithubError::RateLimited)
    ));
    assert!(matches!(
        classify_star(403, Some("42")),
        Err(GithubError::Status(403))
    ));
    assert!(matches!(
        classify_star(502, None),
        Err(GithubError::Status(502))
    ));
}

#[test]
fn only_github_pages_are_opened_for_the_sign_in() {
    assert_eq!(
        trusted_verification_uri("https://github.com/login/device"),
        "https://github.com/login/device"
    );
    assert_eq!(
        trusted_verification_uri("https://evil.example/login"),
        "https://github.com/login/device"
    );
    assert_eq!(
        trusted_verification_uri("http://github.com/login/device"),
        "https://github.com/login/device"
    );
}

#[test]
fn verification_age_follows_the_grace_period_and_rejects_future_dates() {
    let now = 2_000_000_000;
    assert_eq!(grant_at(now).freshness(now), Freshness::Fresh);
    assert_eq!(grant_at(now - GRACE_SECS).freshness(now), Freshness::Fresh);
    assert_eq!(
        grant_at(now - GRACE_SECS - 1).freshness(now),
        Freshness::Stale
    );
    assert_eq!(
        grant_at(now + CLOCK_SKEW_SECS).freshness(now),
        Freshness::Fresh
    );
    assert_eq!(
        grant_at(now + CLOCK_SKEW_SECS + 1).freshness(now),
        Freshness::FromTheFuture
    );
    assert_eq!(grant_at(u64::MAX).freshness(now), Freshness::FromTheFuture);
}

#[test]
fn saved_verification_round_trips_and_is_not_stored_in_the_clear() {
    let dir = temp_state("roundtrip");
    let saved = grant_at(1_700_000_000);
    grant::save(&dir, &saved).expect("save");
    assert!(grant::load(&dir) == Some(saved));
    #[cfg(windows)]
    {
        let raw = std::fs::read(dir.join(grant::FILE_NAME)).expect("raw");
        assert!(!raw.windows(11).any(|w| w == b"gho_example"));
    }
    grant::forget(&dir);
    assert!(grant::load(&dir).is_none());
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[cfg(windows)]
#[test]
fn a_hand_written_or_altered_file_is_ignored() {
    let dir = temp_state("forged");
    let forged = serde_json::to_vec(&grant_at(u64::MAX)).expect("json");
    std::fs::write(dir.join(grant::FILE_NAME), forged).expect("write");
    assert!(grant::load(&dir).is_none());

    grant::save(&dir, &grant_at(1_700_000_000)).expect("save");
    let path = dir.join(grant::FILE_NAME);
    let mut sealed = std::fs::read(&path).expect("read");
    let last = sealed.len() - 1;
    sealed[last] ^= 0xff;
    std::fs::write(&path, sealed).expect("write");
    assert!(grant::load(&dir).is_none());
    std::fs::remove_dir_all(&dir).expect("cleanup");
}
