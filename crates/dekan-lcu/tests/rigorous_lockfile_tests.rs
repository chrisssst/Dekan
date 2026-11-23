use dekan_lcu::lockfile::Lockfile;

struct XorShift64(u64);

impl XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[test]
fn universal_server_lockfile_and_ux_cmdline_parsing() {
    let cases = [
        (
            r#""C:\Riot Games\League of Legends\LeagueClientUx.exe" "--app-port=54321" "--remoting-auth-token=AbC_123-xyz" "--app-pid=9988""#,
            9988,
            54321,
            "AbC_123-xyz",
        ),
        (
            r#""D:\WeGameApps\英雄联盟\LeagueClient\LeagueClientUx.exe" --region=TENCENT --locale=zh_CN --app-port=61234 --remoting-auth-token=cnToken9988 --app-pid=4411"#,
            4411,
            61234,
            "cnToken9988",
        ),
        (
            r#""E:\Garena\LoL\LeagueClientUx.exe" --app-port=12000 --remoting-auth-token=seaTok_77"#,
            777,
            12000,
            "seaTok_77",
        ),
    ];

    for (cmd, fallback_pid, expected_port, expected_token) in cases {
        let parsed = Lockfile::parse_ux_cmdline(cmd, fallback_pid).expect("valid cmdline");
        assert_eq!(parsed.port, expected_port);
        assert_eq!(parsed.auth_token, expected_token);
        assert_eq!(parsed.protocol, "https");
        assert!(parsed.pid > 0);
    }
}

#[test]
fn fuzz_lockfile_and_cmdline_parsers_never_panic() {
    let mut rng = XorShift64(0xCAFE_BABE_1337_0001);
    let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789:=\"'-- \t\r\n/\\_-";

    for _ in 0..1000 {
        let len = (rng.next_u64() % 180) as usize;
        let s: String = (0..len)
            .map(|_| alphabet[(rng.next_u64() as usize) % alphabet.len()] as char)
            .collect();
        let _ = Lockfile::parse(&s); // ignore-ok: fuzz test asserts no panic
        let _ = Lockfile::parse_ux_cmdline(&s, 1234); // ignore-ok: fuzz test asserts no panic
    }
}
