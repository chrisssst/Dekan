# CLAUDE.md — Dekan

Dekan is a League of Legends skin changer for Windows, in Rust. It follows champion select through the client's
local API (LCU), generates the chosen skin from the installed game, builds an overlay of WAD archives and has the
game read it through the LTK injector (`ltk_patcher_host.exe` + `ltk_patcher_dll.dll`). The product in one
sentence: the right skin loads into the game, in every mode, every time. In production; releases are in
`CHANGELOG.md`. `AGENTS.md` points here.

Dekan is independent: no Pengu Loader, no client plugins, nothing read from Rose. The only external dependency is
the LTK injector. It serves many users, so nothing is hardcoded to one drive, folder, server or language.

## Map

```
dekan-core      domain types, AppState (watch channel), phases, supervisor, env names        → nothing
dekan-platform  Win32: processes, discovery, windows, tray, i18n, Slint UI, atomic writes      → core
dekan-wad       WAD v3 reader/writer, BIN/PROP, .fantome, .modpkg, hash index                  → nothing
dekan-lcu       LCU REST + WebSocket, champion select, live selection, skin registration       → core
dekan-classic   store skins and Classic Rift models generated from the installed game          → wad
dekan-inject    mod compatibility, native overlay builder, LTK host, injector trust            → core, platform, wad
dekan-party     encrypted party rooms over a relay                                             → core
dekan-relay     optional self-hosted relay (same protocol as relay-worker/)
dekan-app       binary: composition, lifecycle, tray, catalog, injection trigger              → all
xtask            check, package, installer, audits and probes (xtask/README.md)
installer/       Inno Setup script; .github/ CI, security, auto-merge, release, promote
```

Inside each crate, modules live in segment folders re-exported flat by `lib.rs` (`docs/architecture.md`, "Source
layout"); a new module goes into its segment's folder. Crates with invariants of their own have a `CLAUDE.md`.

## Commands

| Task | Command |
| --- | --- |
| Full gate: fmt, clippy `-D warnings`, tests, error-handling and comment sweeps | `cargo xtask check` |
| Dependencies and licenses | `cargo deny check` |
| One crate / one test | `cargo test -p dekan-wad` / `cargo test -p dekan-app <name>` |
| Release build into `dist\` with `SHA256SUMS` | `cargo xtask package` |
| Installer `dist\installer\Dekan-Setup-<version>-x64.exe` | `cargo xtask installer` |
| Every skin and chroma generated from the installed game | `cargo xtask skin-audit` |
| Remove comments, listing them for the docs | `cargo xtask comments --strip` |

`rtk` filters command output; run anything used as evidence through `rtk proxy <cmd>`.

## Invariants

Each one has broken something before, or protects users or the trust model.

1. The skin is picked in Dekan's own window; nothing runs inside the client, because client plugins were fragile
   and a trust risk.
2. The injector is armed during champion select, before the game process exists: arming late misses the game, and
   a game process older than two seconds is never hooked (a mid-load hook crashed it).
3. Dekan never suspends the game, opens its threads or asks for `SeDebugPrivilege` (ADR-029): it never worked
   against the anti-cheat and it is what antivirus heuristics weigh most.
4. Dekan runs unelevated (`asInvoker`); only `--install-injector` runs elevated, re-verifies the signature and
   copies two files into Dekan's own `tools`. Least privilege, proven in a match.
5. The LCU is the source of truth for champion, phase and owned skins, re-read right before injection. The
   injection target comes from Dekan's own window, never from the LCU.
6. No `unwrap()`/`expect()` in runtime code (tests only), and every `let _ =` on a `Result` carries
   `// ignore-ok: <reason>`, because a swallowed error hides the cause in production (`cargo xtask adr008`).
7. No unsupervised `tokio::spawn`: every task goes through `Supervisor`, so a dead task is reported.
8. State changes only through the named transitions in `dekan-core::state`.
9. Every offset read from a binary format is bounds-checked; out of range is a typed error, never a panic.
10. No comments in code (Rust, Slint, YAML including `run:`, TOML, Inno). A comment is at most one line and only a
    tool directive (`// ignore-ok:`, `# zizmor: ignore[...]`, the version after a pinned SHA). Explanations go to
    `docs/`, a crate README or an ADR.
11. Code, comments, logs and public docs in English. User-facing text goes through `dekan_platform::i18n` or the
    overlay dictionaries; paths come from `dekan_platform::paths` discovery, never a drive letter.
12. Log by intention, never in a loop. `error!` trust broken or operation aborted, `warn!` degradation or refusal,
    `info!` state transition, `debug!` diagnostics (`DEKAN_LOG=debug`).
13. No new third-party dependency without a written justification; `dekan-wad` stays our own parser. Minimizing
    third-party trust is a product goal.
14. Party: relay only, no P2P, XChaCha20-Poly1305 blobs, nothing identifying in clear, anti-spoof against the real
    roster.
15. Version is two numbers (`1.2`) on every user-facing surface; Cargo.toml holds `X.Y.0` as a floor and the
    release stamps the next one (`cargo xtask set-version`). Nobody bumps it by hand.
16. No git commit or push without the maintainer's explicit request; commits and PRs carry no AI attribution.
17. Every fix or improvement gets `.claude/prs/NNN-<type>-<slug>.md` (frontmatter `title`, `labels` from
    `.github/release.yml`, `branch`, `status` open → merged → released; body from
    `.github/pull_request_template.md`, in English). Release notes are built from those files.

Enforced by `cargo xtask check`: 6, 10 and the clippy lints; by a permission prompt: 16. The rest is review.

## Done means

- Any change: `cargo xtask check` passes; `cargo deny check` too when dependencies changed.
- Architecture change or new dependency: an ADR records the decision and its reason.
- `.github/` change: actionlint and zizmor pass locally (skill `github-pipeline`); every `uses:` pinned to a SHA.
- Anything that touches the game: proof from a real match and its log. The modes that define "done":

| Mode | Known trap |
| --- | --- |
| Normal/Ranked draft | A lock in the last second can miss the 900 ms arming debounce |
| Blind pick | No completed actions |
| ARAM | Champion changes after lock (bench swap) |
| Swiftplay / Quickplay / Brawl | No champion select: champions are picked in the lobby and both are armed at once |
| Arena | Two champions in the same cell group |
| Rotating (URF etc.) | Own carousel |
| Classic Rift (JADE) | Offset ids 60000/60000000; never force the base |
| Reconnect / post-dodge | The patcher keeps running through the game's exit and rescans |

## Release flow

An approved PR into `main` gets squash auto-merge once "CI OK" and "Security OK" pass. `release.yml` picks the next
version (last tag + 1 minor, next major with the `breaking` label; docs-only merges skip), builds, attests and
publishes a pre-release; after a match test `promote.yml` marks it latest. Protected paths (workflows, installer,
xtask, toolchain, lockfile, trigger, injector trust and code, party cipher) never auto-merge.

## Where to look

| Need | Where |
| --- | --- |
| Architecture, startup, shared state, discovery | `docs/architecture.md` |
| End-to-end flows (skin, custom mod, Classic Rift, party, patch, recovery) | `docs/flows.md` |
| Toolchain, tests under Wine, CI/CD, releases | `docs/build-and-ci.md` |
| Log policy and the events to look for | `docs/observability.md` |
| What is injected, trust anchors, risk | `docs/security.md` |
| Each crate's files and responsibilities | `crates/<crate>/README.md` |
| `cargo xtask` commands and probes | `xtask/README.md` |

`.claude/` and `.agents/` are local and ignored by git; public documentation is `README.md`, `docs/` and the crate
READMEs, in English, without internal references.
