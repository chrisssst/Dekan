# Dekan update integration

The update imports source v1.6 (`3cfa3d76646837a8ac797402afeca42c147f9ec1`) into Dekan main
(`643841027ffc19f4565bad7e1af764b121e22a58`). Upstream v1.6 is a prerelease.

## Resulting behavior

- The control panel uses Home, Options, Party and Help tabs. Dekan's Classic, Crimson and Obsidian theme
  choices remain in Options, with the existing saved theme preference. The champion-selection overlay,
  welcome window and party dialogs initialize from the saved theme and follow changes while open.
- Lobby invitations and reconnect skin selection follow the upstream implementation. Injector failures and
  exported diagnostics carry the upstream failure reporting and redaction changes.
- Dekan's logo, application and package names, local data paths, release URLs, existing Discord Rich Presence,
  Turkish default and English dictionary are retained. The new text fields have Turkish translations.
- The existing nonblocking installer handoff remains: Dekan starts the verified installer and exits so its
  installed files can be replaced. Upstream signature verification remains enabled.
- The dependency updates, relay changes, archive safety checks and CI workflow changes are included.
- Cargo's version floor is unchanged. The existing release workflow selects the next Dekan version.

The default party relay is `wss://dekan-party-relay.heeazk.workers.dev`. Existing `party.json` files using the
previous built-in default migrate to this address without losing additional fields. Custom relay URLs and
explicit environment overrides stay unchanged. The previous default is identified only by a SHA-256 fingerprint for migration; its obsolete product
label and hostname are not embedded in the application.

## Validation

Passed:

- `cargo fmt --all -- --check` and `git diff --check`.
- Actual xtask static sweep modules run through a native harness: all discarded Results justified, no code
  comments, no source file above 400 lines and no source directory above five code files. Panel theme
  regression tests live separately from the main panel tests to meet the same limit enforced by Windows CI.
- `cargo test -p dekan-core -p dekan-wad -p dekan-classic --target x86_64-unknown-linux-gnu --locked`:
  237 tests passed, none failed.
- Clippy with `-D warnings` for those three crates, all targets, on the Linux host target.
- `cargo check -p dekan-platform --all-targets --target x86_64-pc-windows-msvc --locked`, including the Slint
  interface and both locale dictionaries; Clippy with `-D warnings` also passed for this Windows target.
  Theme-button and overlay theme propagation regression tests are compiled, but their Windows UI runtime
  has not been executed.
- Party crate: 28 tests passed and Clippy with warnings denied on the Linux host target, including saved
  relay migration, custom relay preservation and malformed-file preservation.
- The deployed Dekan relay returned HTTP 200 with its service identity; two WebSocket clients joined one
  random test room and received a selection-clear update.
- Relay worker TypeScript check, 22 tests and npm audit (zero reported vulnerabilities).
- actionlint 1.7.12; zizmor 1.31.1 offline (no findings, existing suppressions retained).
- cargo-deny 0.20.2: advisories, bans, licenses and sources passed. It reported stale advisory-ignore warnings.
- Existing logo and icon bytes match Dekan main. No unsupported locale modules were reintroduced.

Not yet verified:

- The full workspace MSVC build/check is blocked on this Linux host: `ring` requires the Windows C tools
  (`lib.exe`). This is an environment limitation, not a passing build. `cargo xtask check` and Windows CI
  still need to pass before release.
- No Windows installer was built or run, no new release was published, and no live match was tested.
- Lobby party/reconnect behavior and the reported Zed crash require a real game and its logs. This update
  does not establish that the Zed crash is fixed.

## Review

The update is prepared on branch `update-1.6`. Windows CI and live-game verification remain required before release.
