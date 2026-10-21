# Contributing to Dekan

Thanks for helping. Dekan is a small project with a high bar for anything that touches the game, so this
page explains what makes a contribution easy to accept.

## Before you start

- **Questions and ideas:** ask on [Discord](https://discord.gg/dekan) first. A quick chat often saves a
  pull request that heads in the wrong direction.
- **Bugs:** open an issue with the bug template and **attach the log** from `%LOCALAPPDATA%\Dekan\logs` for
  the match where it failed.
- **Security issues:** never in public. See [SECURITY.md](SECURITY.md).

## Development setup

1. Windows 10/11 x64, [Rust](https://rustup.rs) (the toolchain in `rust-toolchain.toml` installs itself) and,
   for the installer, [Inno Setup 6](https://jrsoftware.org/isinfo.php).
2. `cargo build` and `cargo xtask check` must pass before you open a pull request, and `cargo deny check` too
   when dependencies changed.
3. Read the README of the crate you are changing (`crates/<crate>/README.md`) and the relevant page in
   [`docs/`](../docs/README.md).

## Rules the code follows

- No `unwrap()` or `expect()` outside tests. Errors are handled or propagated with context.
- Every discarded `Result` (`let _ = …`) carries `// ignore-ok: <why this is safe>`.
- Background tasks go through the `Supervisor`, never a bare `tokio::spawn`.
- The shared state only changes through the named transitions in `dekan-core::state`.
- Binary parsing checks every offset; bad input returns a typed error and never panics.
- Code, logs and docs are in English. Text shown to users goes through the translation tables.
- No comments in code files (Rust, HTML/CSS/JS, TypeScript, YAML, TOML, the installer script). Names, types
  and tests say what the code does; the reason behind it goes in `docs/`, the crate README or a decision
  record. The only comments allowed are one-line tool directives: `// ignore-ok: <why>`, `# zizmor: ignore[...]`
  and the version after a pinned action SHA. `cargo xtask check` enforces this, and
  `cargo xtask comments --strip` removes the rest after proving the code is unchanged.
- No code file over 400 lines and no folder with more than 5 code files, tests included. `cargo xtask check`
  enforces both.
- No hardcoded drive letters, folders, servers or languages.
- Log state changes, never inside a loop.
- A new dependency needs a short written justification in the pull request.

## Pull requests

- Keep them focused: one change, with the reason explained in the description.
- Fill in the checklist in the pull request template.
- **Anything that changes what happens in game** (injection, game modes, party mode) must include evidence
  from a real match: the Dekan log, and what you saw. Passing tests is not enough for these changes.
- CI must be green. After a maintainer approves, the pull request merges automatically once every required
  check passes. Changes to workflows, the installer, `xtask`, the toolchain and dependency policy, the lockfile, the
  injection trigger, the injector code and its trust check, the party cipher or the relay's deploy config are
  merged by hand.

## Releases

Nobody bumps the version by hand. The merge publishes a pre-release whose version is the last release plus one
minor (the next major for a pull request labelled `breaking`), and it becomes the latest release only after it has
been tested in a real match. See [docs/build-and-ci.md](../docs/build-and-ci.md).

## License

By contributing, you agree that your contributions are licensed under the [MIT License](../LICENSE).
