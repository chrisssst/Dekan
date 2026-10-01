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
2. `cargo build` and `cargo xtask check` must pass before you open a pull request.
3. Read the README of the crate you are changing (`crates/<crate>/README.md`) and the relevant page in
   [`docs/`](docs/README.md).

## Rules the code follows

- No `unwrap()` or `expect()` outside tests. Errors are handled or propagated with context.
- Every discarded `Result` (`let _ = …`) carries `// ignore-ok: <why this is safe>`.
- Background tasks go through the `Supervisor`, never a bare `tokio::spawn`.
- The shared state only changes through the named transitions in `dekan-core::state`.
- Binary parsing checks every offset; bad input returns a typed error and never panics.
- Code, comments, logs and docs are in English. Text shown to users goes through the translation tables.
- No hardcoded drive letters, folders, servers or languages.
- Log state changes, never inside a loop.
- A new dependency needs a short written justification in the pull request.

## Pull requests

- Keep them focused: one change, with the reason explained in the description.
- Fill in the checklist in the pull request template.
- **Anything that changes what happens in game** (injection, game modes, party mode) must include evidence
  from a real match: the Dekan log, and what you saw. Passing tests is not enough for these changes.
- CI must be green. After a maintainer approves, the pull request merges automatically once every required
  check passes. Changes to workflows, the installer, trusted hashes, injector code or the party cipher are
  merged by hand.

## Releases

Maintainers bump the version in the root `Cargo.toml`. The merge publishes a pre-release, and it becomes the
latest release only after it has been tested in a real match. See [docs/build-and-ci.md](docs/build-and-ci.md).

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
