# Dekan engineering guide

This folder is for people who maintain Dekan's code. It covers how the pieces fit together, how data moves
from champion select to the game, and how changes are built, tested and shipped. The user guide is the
[README](../README.md) at the repository root.

## Dekan in one paragraph

Dekan is a Windows tray application written in Rust. It watches the League client through its local API,
generates the chosen skin from the game installed on the machine, and builds an overlay of modified game
archives. An injector DLL then makes the game read that overlay instead of its own files. Dekan runs
without administrator rights, never writes to the game folder, and releases every external resource it
acquires through an owning guard.

## Contents

| Document | What it covers |
| --- | --- |
| [architecture.md](architecture.md) | The crates and their dependency rules, startup sequence, shared state, runtime discovery |
| [flows.md](flows.md) | End to end: a store skin, a custom mod, Classic Rift, party mode, patches, shutdown and recovery |
| [build-and-ci.md](build-and-ci.md) | Toolchain, local checks, tests, the CI and security pipeline, automated releases |
| [observability.md](observability.md) | Where logs go, what each level means, which events to look for |
| [security.md](security.md) | What is actually injected, how binaries are trusted, honest risk, code invariants |

Each crate also has its own README with its files and responsibilities: [dekan-core](../crates/dekan-core),
[dekan-platform](../crates/dekan-platform), [dekan-wad](../crates/dekan-wad),
[dekan-lcu](../crates/dekan-lcu), [dekan-classic](../crates/dekan-classic),
[dekan-inject](../crates/dekan-inject), [dekan-party](../crates/dekan-party),
[dekan-relay](../crates/dekan-relay), [dekan-app](../crates/dekan-app), and the build tool
[xtask](../xtask).

## Principles the code follows

1. **The client knows the truth.** Champion, phase and owned skins are read again from the client right
   before anything is built, never taken from a stale copy. The skin to inject, however, always comes from
   Dekan's own window, because the client has no way to express a skin you do not own.
2. **No panics at runtime.** `unwrap()` and `expect()` only appear in tests. Every discarded `Result` carries
   a comment that explains why it is safe to ignore, and a check enforces this.
3. **Least privilege.** Dekan never asks for administrator rights and never writes to the game folder.
   External resources (a suspended process, a child process, a temporary file) are released by `Drop`, even
   during a panic.
4. **Nothing is hardcoded to one machine.** The game path is discovered, the language follows the client,
   and servers can be configured. All configuration lives in `dekan_core::env`.
5. **Logs record changes.** A line repeated inside a polling loop is treated as a bug.

## Checking a change

```powershell
cargo xtask check    # error-handling sweep, rustfmt, clippy -D warnings, all tests
cargo deny check     # advisories, licenses, banned crates, sources
```

Some checks cannot be automated: a change that affects what happens inside the game (injection, game
modes, party mode) is only finished once it has been seen working in a real match, with the log to prove
it. A green test run is not that proof.
