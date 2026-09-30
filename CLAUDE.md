# CLAUDE.md — AI context for Dekan

> **Read this first.** This file is the single entry point for any AI working on this project. `AGENTS.md`
> points here.

## What is Dekan?

Dekan is a **League of Legends skin changer for Windows**, written in Rust as a rewrite of Rose (Python).
It follows champion select through the League client's local API, generates the chosen skin from the
installed game, builds an overlay of modified WAD archives and has the game read it through the LTK
injector (`ltk_patcher_host.exe` + `ltk_patcher_dll.dll`).

**The product in one sentence:** the right skin loads into the game, in every mode, every time.

**Product direction:** Dekan is independent. No Pengu Loader, no client plugins, nothing read from Rose.
The only external dependency is the LTK injector, slated for a Rust reimplementation. It must serve many
users: no hardcoded drive, folder, server or language.

## Where things are

| Need | Where |
| --- | --- |
| Architecture, startup, shared state, discovery | `docs/architecture.md` |
| End-to-end flows (skin, custom mod, Classic Rift, party, patch, recovery) | `docs/flows.md` |
| Toolchain, tests under Wine, CI/CD, auto-merge, releases | `docs/build-and-ci.md` |
| Log policy and events to look for | `docs/observability.md` |
| What is injected, trust anchors, risk, invariants | `docs/security.md` |
| Each crate's files and responsibilities | `crates/<crate>/README.md` |
| `cargo xtask` commands and probes | `xtask/README.md` |
| GitHub pipeline setup and operation | skill `github-pipeline` (local, `.claude/skills/`) |

`.claude/` and `.agents/` are local and ignored by git. Public documentation lives in `README.md`, `docs/`
and the crate READMEs, in English, without internal references.

## Status (2026-09-25, version 1.0.0)

- Injection **proven in a live match** on patch 16.19 (Draft/Ranked), native overlay builder + LTK injector,
  Dekan running **unelevated**.
- Blind, ARAM, Swiftplay, Arena, rotating modes and reconnect are implemented but **not yet proven in game**.
- Party mode works against the public relay; not yet proven with several players in one match.
- **The LTK DLL refuses game builds newer than 2026-10-04 07:00Z** (`0x6ac1f970`, checked on the game exe's
  `TimeDateStamp`). A refreshed DLL is needed for the first patch built after that.
- Known gaps: large custom mods are slow to build inside champion select; skin packages are generated per
  patch; no "disable mods" option on the reconnect screen yet.

## Architecture

```
dekan-core      → Domain types, AppState (watch channel), phases, supervisor, env names
dekan-platform  → Win32: processes, game/tools discovery, windows, tray, i18n, atomic writes
dekan-wad       → WAD v3 reader/writer, BIN/PROP, .fantome, hash index
dekan-lcu       → LCU REST + WebSocket, champion select, live selection, skin registration
dekan-classic   → Store skins and Classic Rift models generated from the installed game
dekan-inject    → Mod compatibility, native overlay builder, LTK host, suspension guard
dekan-party     → Encrypted party rooms over a relay
dekan-relay     → Optional self-hosted relay (same protocol as relay-worker/)
dekan-app       → Binary: composition, lifecycle, tray, catalog, injection trigger
```

**Dependency rules:** core→nothing, platform→core, wad→nothing, lcu→core, classic→wad,
inject→core+platform+wad, party→core, app→all.

## Decisions that must not be undone

| Decision | Why |
| --- | --- |
| The skin is picked in Dekan's own window; nothing runs inside the client | Client plugins (Pengu) were fragile and a trust risk |
| The injector is armed during champion select, before the game process exists | Arming late misses the game; suspending the game is denied by Vanguard and only remains a fallback |
| The overlay is built natively and byte-faithful: unchanged entries keep the game's exact compressed bytes | Patch 16.19 rejects recompressed archives as corrupt (`Map11.wad.client`) |
| A champion mod never changes a path a map WAD also holds | Changing only one side made the game report the champion WAD as inconsistent and open the repair screen |
| Dekan runs unelevated (`asInvoker`) | Least privilege; proven to work in a match |
| Third-party binaries only from Dekan's own folder, SHA-256 checked against `AUDITED_*_HASH` in `dekan-app::trigger` | Hash is the trust anchor; the byte-patched "2040" DLL failed in a match and is refused |
| `dekan-wad` stays our own parser (no `cdragon-*`) | Full control over all five entry types and bounds checks |
| Party: relay only, no P2P, XChaCha20-Poly1305 blobs, nothing identifying in clear, anti-spoof against the real roster | Privacy and safety of other players |
| No new third-party dependency without a written justification | Minimizing third-party trust is a product goal |

## Critical rules

1. **Source of truth for champion, phase and owned skins = LCU**, re-read immediately before injection. The
   injection target comes from Dekan's own window, never from the LCU.
2. **No `unwrap()`/`expect()` in runtime code.** Tests only.
3. **No unsupervised `tokio::spawn`.** All tasks go through `Supervisor`.
4. **Every `let _ =` on a `Result` needs `// ignore-ok: <reason>`** (`cargo xtask adr008` enforces it).
5. **State changes only through named transitions** in `dekan-core::state`.
6. **Every offset is bounds-checked** when reading binary formats. Out of range = typed error, never a panic.
7. **Code, comments, logs and public docs in English.**
8. **Log by intention, never in a loop.** INFO by default (`DEKAN_LOG=debug` for detail). `error!` = trust
   broken or operation aborted, `warn!` = degradation or refusal, `info!` = state transition, `debug!` =
   diagnostics.
9. **User-facing text goes through `dekan_platform::i18n`** or the overlay dictionaries; paths come from
   `dekan_platform::paths` discovery. Never a literal in one language, never a drive letter.
10. **The LTK injector lives in Dekan's own folder, is hash-validated, and its absence tells the user.**
    Never patch the DLL bytes or strip its signature. **Never bundle it:** the LTK Patcher License forbids
    redistributing League Toolkit's signed binaries outside an official LTK Manager release, so users copy
    host + DLL from that release into `tools\` (the README also links a convenience mirror, `tools.zip` on
    chosen by the maintainer). The audited hashes match **LTK Manager 1.21.0 and 1.22.0**
    (`src-tauri/resources/`, installed to `%LOCALAPPDATA%\LTK Manager`). Changing `AUDITED_*_HASH` means
    updating the version and hashes in the README's "Step 2 — Add the injector".
11. **Never perform git commit or git push without explicit user approval.** Code modifications are made,
    tested, and presented to the user; commits/pushes are executed only when the user explicitly requests it.
    Commits and PRs never carry `Co-Authored-By` or any AI attribution trailer.
12. **Version format is two numbers (`1.0`, `1.1`, `2.0`) across UI, installer, and release tags.**
    Cargo strictly requires SemVer 2.0.0 (`MAJOR.MINOR.PATCH`), so Cargo.toml bumps minor (e.g. `1.1.0`)
    and helper functions (`display_version()`) strip the zero for user-facing surfaces.
13. **Overlay keyboard focus goes to the WebView, never to the host window.** The UI asks for focus over
    IPC; Rust takes the foreground (thread-input attach) and then calls `webview.focus()`. `SetFocus` on the
    host HWND pulls focus out of the WebView2 child and the search box stops receiving keys.
14. **A retargeted skin bin keeps the source bin's links.** `retarget_skin_bin` re-keys only the skin
    object and its Resources; everything they reference by hash (animation graph, VFX, shared bins) lives in
    the source bin's dependencies, so `skin0.bin` links `SkinN.bin` **and** every link `SkinN.bin` had.
    Dropping them left legendary/mythic skins on the base graph (God-King Garen's E: frozen sword).
    Not yet proven in a match.
15. **No comments in code.** No `//`, `///`, `/* */`, `<!-- -->` or `#` comments in Rust, HTML/JS, YAML,
    TOML or scripts: names, types and tests carry the meaning; the "why" goes in `docs/`, crate READMEs or
    ADRs. The only exception is the `// ignore-ok: <reason>` marker required by rule 4.

## Before making changes

1. Read the relevant crate README and `docs/` page.
2. Changing architecture or adding a dependency → record the decision and its reason.
3. Run: `cargo xtask check && cargo deny check` (fmt, clippy `-D warnings`, tests, error-handling sweep).
4. Changing `.github/` → run actionlint and zizmor locally (see the `github-pipeline` skill); every `uses:`
   pinned to a commit SHA.
5. Anything that touches the game closes only with **proof from a real match** and its log.
6. `rtk` filters command output. For anything used as evidence, run `rtk proxy <cmd>`.

## Release flow

A PR that bumps `[workspace.package].version` → maintainer approval → `automerge.yml` enables squash
auto-merge once "CI OK" and "Security OK" pass → `release.yml` builds, attests and publishes a
**pre-release** → tested in a match → `promote.yml` marks it latest. Protected paths (workflows, installer,
xtask, toolchain, lockfile, trigger/hashes, injector code, party cipher) never auto-merge.

## Game modes matrix (defines "done")

| Mode | Specifics | Known trap |
| --- | --- | --- |
| Normal/Ranked (draft) | Base path | A lock in the last second can miss the 900 ms arming debounce |
| Blind pick | No bans | No completed actions |
| ARAM | Bench swap | Champion changes after lock |
| Swiftplay | Overlay built outside the handler | Rose needs a GameStart fallback |
| Arena | Doubles | Two champions in the same cell group |
| Rotating (URF etc.) | Own carousel | Where Rose's scraper dies |
| Classic Rift (JADE) | WAD-generated mod | Offset ids 60000/60000000; never force the base |
| Reconnect / post-dodge | Re-injection | The patcher must keep running through the game's exit and rescan |
