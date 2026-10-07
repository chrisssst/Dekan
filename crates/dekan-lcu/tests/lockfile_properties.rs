use dekan_lcu::lockfile::Lockfile;

#[test]
fn any_accepted_lockfile_only_ever_points_at_the_local_client() {
    let mut state = 0x5EED_0301u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let names = ["LeagueClient", "", "Riot Client", "x:y"];
    let pids = ["4242", "0", "-1", "999999999999", "", "12a"];
    let ports = ["51234", "0", "1", "65535", "65536", "-5", "", "8080 "];
    let tokens = [
        "s3cr3t",
        "",
        "a\r\nb",
        "@evil.com",
        "/x",
        "tok en",
        "riot:pw",
    ];
    let protocols = ["https", "http", "", "wss"];
    let mut accepted = 0;
    for _ in 0..50_000 {
        let mut pick = |list: &[&'static str]| list[(next() % list.len() as u64) as usize];
        let mut text = [
            pick(&names),
            pick(&pids),
            pick(&ports),
            pick(&tokens),
            pick(&protocols),
        ]
        .join(":");
        if next() % 4 == 0 {
            let at = (next() as usize) % (text.len() + 1);
            if text.is_char_boundary(at) {
                text.insert(at, [':', ' ', '\n', '@'][(next() % 4) as usize]);
            }
        }
        if let Ok(lockfile) = Lockfile::parse(&text) {
            accepted += 1;
            assert_ne!(lockfile.port, 0, "{text:?}");
            assert!(!lockfile.auth_token.is_empty(), "{text:?}");
            assert_eq!(
                lockfile.base_url(),
                format!("https://127.0.0.1:{}", lockfile.port)
            );
            assert_eq!(
                lockfile.ws_url(),
                format!("wss://127.0.0.1:{}", lockfile.port)
            );
            assert!(lockfile.basic_auth_header().starts_with("Basic "));
            assert!(
                !lockfile.basic_auth_header().contains(['\r', '\n']),
                "{text:?}"
            );
        }
    }
    assert!(accepted > 0, "the generator must reach valid lockfiles too");
    let real = Lockfile::parse("LeagueClient:4242:51234:s3cr3t:https\r\n").expect("real");
    assert_eq!(
        (real.pid, real.port, real.auth_token.as_str()),
        (4242, 51234, "s3cr3t")
    );
    assert!(Lockfile::parse("LeagueClient:4242:0:s3cr3t:https").is_err());
}
