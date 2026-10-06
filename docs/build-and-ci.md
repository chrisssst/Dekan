# Build, tests and CI

## Toolchain

Rust stable, edition 2024, pinned in `rust-toolchain.toml`. CI reads the same file, so a developer machine
and CI cannot drift apart. The release target is `x86_64-pc-windows-msvc` with a static C runtime. Every
crate inherits its version from `[workspace.package]` in the root `Cargo.toml`.

## Local checks

```powershell
cargo xtask check    # error-handling sweep, no comments in code, rustfmt, clippy -D warnings, all tests
cargo deny check     # advisories, licenses, banned crates, sources
```

`cargo xtask check` runs, in order:

1. the error-handling sweep (every discarded `Result` must carry a written justification),
2. the comment check (`cargo xtask comments`: no comment in a tracked code file other than a one-line tool
   directive),
3. `cargo fmt --all -- --check`,
4. `cargo clippy --workspace --all-targets -- -D warnings`,
5. `cargo test --workspace`.

## Running the tests without Windows

On Linux, the tests can run for the Windows target under Wine by building for `x86_64-pc-windows-gnu`. Two
things are needed:

1. `WebView2Loader.dll` on `WINEPATH`, because the `wry` crate links against it. Without it, the test binaries
   that depend on `dekan-platform` fail to load (`status c0000135`), which is not a real failure.
2. A virtual display (`xvfb`) for the Win32 window calls.

```sh
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=<script that runs wine with WINEPATH set>
xvfb-run -a cargo test --workspace --target x86_64-pc-windows-gnu
```

This validates the logic on the Windows target. It does not replace a real match: injection, game modes and
party mode are only proven in game.

## Test layout

Large test modules live in sibling files named `<module>_tests.rs`, included with
`#[cfg(test)] #[path = "..."] mod tests;`. They are still the module's own `tests` module with
`use super::*`, so they see private items. The split only makes files easier to navigate.

## Test layers

What a test can prove depends on what it runs. Dekan keeps three layers apart and never lets a lower one
claim what only a higher one can show.

| Layer | What it runs | What it proves | Where |
| --- | --- | --- | --- |
| Assets and runtime | Dekan's own generator, mod import and staging, compatibility check and overlay builder, on a synthetic game install built by the test | The chosen skin, its companions and a custom mod end up in the archives the game would open: slot 0 holds the skin's own definition, every link resolves, the animation graph the skin names is reachable, archives that share a path agree, headers and untouched bytes are the game's, output is deterministic | `crates/dekan-app/tests/skin_pipeline.rs` (runs everywhere, including CI) |
| Same, on the installed game | The same production code against a real install | The above for every champion, skin and chroma of the current patch | `cargo xtask harness`, `cargo xtask skin-audit`, `native_overlay_faithful` (ignored; needs the game) |
| Interface | Dekan's windows and pages in Edge, and the real WebView2 window | Pages render, react and pass accessibility checks; a release changes only what it meant to | `cargo xtask ipc-probe` and the page dumps (`DEKAN_UI_DUMP`) used by the visual tests |
| Real match | The game engine | How the skin looks and animates in game: models, animations, shaders, particles | Manual, with the log; the only layer that can show it |

The first two layers show that Dekan loads and applies the skin files correctly. They do not render
anything: no test here can show how a skin looks or animates in the game, because that needs the game engine.

## Pipeline

All workflows live in `.github/workflows`. Every third-party action is pinned to a commit SHA, every job
starts with read-only permissions, and no workflow runs pull request code with a write token.

### On every push and pull request

| Workflow | Job | What it proves |
| --- | --- | --- |
| `ci.yml` | Format, lint, test | `Cargo.lock` is current, and `cargo xtask check` passes on Windows |
| | Release build | The release profile builds, the executable carries its file details, and its manifest runs it without elevation |
| | Dependencies | `cargo deny`: no known advisory, no disallowed license, no banned crate or unknown source |
| | Relay worker | `npm audit` and a strict TypeScript typecheck |
| | Workflow lint and audit | `actionlint` (syntax, expressions) and `zizmor` (template injection, credential leaks, unpinned actions) |
| | **CI OK** | Aggregate: fails if any job above failed, was cancelled or skipped |
| `security.yml` | CodeQL | Static analysis of Rust, TypeScript and the workflows themselves (`security-extended` queries) |
| | Secret scan | `gitleaks` over the whole history, since a deleted secret is still public |
| | Dependency review | On pull requests: blocks new dependencies with known vulnerabilities |
| | **Security OK** | Aggregate of the jobs above |

`security.yml` also runs weekly, so new advisories and queries reach code that has not changed.

### From approval to release

```text
maintainer approves a pull request
  └─ pr-approved.yml records the PR number and the approved commit (read-only token)
     └─ automerge.yml re-checks everything through the API and enables auto-merge (squash)
        └─ GitHub merges only when "CI OK" and "Security OK" pass on that exact commit
           └─ release.yml: if the workspace version has no release yet, full gate without cache,
              dekan.exe signed, installer built around it and signed (when signing is configured),
              checksums,
              build provenance attestation → published as a PRE-RELEASE
              └─ after testing it in a real match, a maintainer runs promote.yml, which checks
                 the checksums and the attestation and marks the release as latest
```

**What auto-merge refuses**, even with an approval:

- a commit pushed after the approval,
- a draft, a pull request not targeting `main`, or one with changes still requested,
- an approval from someone without write access,
- the `no-automerge` label,
- any change to paths that decide trust, permissions or what ships: `.github/`, `installer/`, `xtask/`,
  `.cargo/`, the toolchain, `deny.toml`, `Cargo.lock`, the app's build script and injection trigger, the
  injector host and DLL validation, the party cipher and token, and the relay's deploy
  config. These are merged by hand.

The `main` branch ruleset (`.github/rulesets/main.json`) enforces the rest: pull requests only, squash merges,
a code owner's approval, stale approvals dismissed on push, required checks up to date with `main`, no force
pushes or deletions.

### Why the workflows are shaped this way

- **CI runs on every branch push**, not only on `main`: a build handed to testers passes the same gate before
  it reaches them.
- **The toolchain comes from `rust-toolchain.toml`**, the file every local build reads, so CI and a developer
  machine cannot drift apart. `cargo metadata --locked` fails a change to `Cargo.toml` that did not update
  `Cargo.lock`, which would otherwise build with versions nobody reviewed.
- **The release build is its own job** because the release profile (LTO, static CRT, embedded resources) is
  what users run; a debug-only gate misses a broken build script or a missing manifest. It then checks the
  file details and the `asInvoker` manifest that `build.rs` embeds: without them the executable looks
  anonymous in Explorer and could prompt for elevation. It builds with `cargo xtask package`, like the release,
  and refuses an executable that needs the Visual C++ runtime, carries the build machine's paths or has no PE
  checksum.
- **The relay worker is type-checked** because a type error there breaks party mode for every user without a
  single Rust change.
- **Workflows are linted like code** because they run with repository permissions.
- **`CI OK` and `Security OK` are aggregates** so branch protection requires one stable name whatever jobs are
  added.
- **Auto-merge is split in two.** A review event from a fork only gets a read-only token, so `pr-approved.yml`
  does nothing privileged: it records the pull request and the approved commit as an artifact.
  `automerge.yml` runs in the base repository, treats that artifact as untrusted input, re-checks everything
  through the API (the latest review of each reviewer counts, the approval must come from someone with write
  access, nobody may still request changes) and only *enables* auto-merge; GitHub merges once the required
  checks pass on that exact commit. It uses a GitHub App token because a merge made with the default token does
  not trigger the release workflow.
- **The release repeats the full gate without cache** on the exact commit being shipped; no build output is
  reused from another run. Write permission exists only in the job that creates the tag and the release.
- **Promotion is the one manual step.** It needs the `production` environment and refuses an installer this
  repository's release workflow did not build.
- **The secret scan reads the whole history**, since a key committed and then deleted is still public. The
  gitleaks allowlist covers only fixed test fixtures that look like keys.
- **Dependabot** bumps the pinned action SHAs, which would otherwise never move, and only version bumps of
  existing crates (a new dependency still needs a written justification). The RustCrypto crates share
  `hybrid-array`/`crypto-common`, so a major bump of one only builds with the others: they arrive together.

Anyone can check where an installer was built:

```powershell
gh attestation verify Dekan-Setup-<version>-x64.exe --repo Isllanrx/Dekan
gh attestation verify Dekan-Setup-<version>-x64.exe --bundle Dekan-Setup-<version>-x64.exe.sigstore.json --repo Isllanrx/Dekan
```

The second form works offline with the Sigstore bundle published next to the installer.

### Dependency policy (`deny.toml`)

- Licenses not in `allow` are denied. `webpki-roots` ships Mozilla's CA bundle as data under
  CDLA-Permissive-2.0 (permissive, no copyleft); `ryu` comes in through `serde_json`.
- Duplicate versions only warn: they are transitive (`getrandom`, `syn`, `thiserror`, `webpki-roots`, with
  rustls, reqwest and tungstenite moving at different speeds), and denying them would keep CI red for
  something no change here can fix.
- Wildcard versions are allowed only for the workspace's own path dependencies.
- Vulnerabilities and unsound advisories are always denied (cargo-deny 0.18 removed the per-severity keys).

### Repository settings the pipeline needs

| Setting | Why |
| --- | --- |
| GitHub App with *Contents* and *Pull requests* write, installed on the repository; `DEKAN_BOT_APP_ID` variable and `DEKAN_BOT_PRIVATE_KEY` secret | A merge made with the default token would not trigger the release workflow |
| *Allow auto-merge* and *Allow squash merging* | Auto-merge waits for the required checks |
| The ruleset, applied with `gh api -X POST repos/<owner>/<repo>/rulesets --input .github/rulesets/main.json` | Makes the checks and reviews mandatory |
| `production` environment with a required reviewer | Gates promotion to latest |
| Approval required for workflows from outside collaborators | First-time contributors cannot run workflows unreviewed |
| `SCORECARD_TOKEN` secret: fine-grained token for this repository only, *Administration: Read-only* | The default token cannot read rulesets, so the Scorecard Branch-Protection check scores 0 even when the ruleset exists |
| SignPath project; `SIGNPATH_API_TOKEN` secret; `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`, `SIGNPATH_EXE_CONFIGURATION_SLUG` and `SIGNPATH_SETUP_CONFIGURATION_SLUG` variables | Authenticode signing of `dekan.exe` and the installer. Without the secret the signing steps are skipped and the release ships unsigned, which Windows SmartScreen and antivirus reputation treat as an unknown program |

## Packaging

- `cargo xtask package` builds the release, copies it to `dist\` and writes `SHA256SUMS`. It owns the release
  compiler flags: the C runtime is linked statically and the build machine's paths (Cargo home, toolchain,
  workspace) are rewritten, whatever `RUSTFLAGS` holds. A set `RUSTFLAGS` (CI uses `-D warnings`) otherwise
  replaces the flags in `.cargo/config.toml`, which silently dropped the static runtime: the executable then
  needed the Visual C++ runtime and carried the builder's user folder in its panic messages. `build.rs`
  also asks the linker for the PE checksum.
- The LTK injector is **not** packaged. Its license does not allow other projects to redistribute League
  Toolkit's signed binaries, so users copy `ltk_patcher_host.exe` and `ltk_patcher_dll.dll` from an official
  LTK Manager release into `Program Files\Dekan\tools`. Dekan only accepts them if their SHA-256 matches the
  audited hashes in `dekan_app::trigger`. `xtask` reads the same constants for the install audit.
- `cargo xtask installer` passes the workspace version to Inno Setup (`installer/dekan.iss`) and produces
  `Dekan-Setup-<version>-x64.exe`. With `--prebuilt` it packages the `dist\dekan.exe` already there instead
  of rebuilding it, so the release can sign the binary before it goes into the installer.
- The release signs with Authenticode through SignPath. The Sigstore attestation proves where a build came from
  to anyone who checks it with `gh`, but Windows does not read it: only an Authenticode signature counts for
  SmartScreen and antivirus reputation.
- The skin library is not shipped. Store skins are generated from the installed game on each patch, and the
  installer only creates an empty `library` folder.
- Files are listed one by one in the installer, never with a wildcard.

### Installer decisions (`installer/dekan.iss`)

- **Version**: passed by `cargo xtask installer` from `Cargo.toml`, so the installer cannot claim a version the
  binary does not have; the fallback in the script only applies when ISCC is run by hand. `VersionInfoVersion`
  accepts numbers only, so a pre-release suffix is cut there and kept in the text versions.
- **Location**: `Program Files`, never `%LOCALAPPDATA%\Programs`. Nothing that touches the game may live in a
  folder a normal user can write to, and the injector lives under `{app}\tools`, which only an administrator
  can change. With `PrivilegesRequired=lowest`, `{autopf}` would send the whole install to LocalAppData, and
  an old per-user install path is never inherited on upgrade.
- **Upgrades**: Setup checks the mutex Dekan holds for its whole life and asks the user to close it, instead
  of failing on a file in use or asking for a reboot. It moves an old per-user install out, deletes backup
  copies in `tools` (never loaded, and they confuse the hash check) and drops overlays built by an earlier
  version: the overlay cache is keyed by builder revision, so this frees gigabytes and costs one build.
- **Nothing third-party or stale is shipped**: no injector (see above), no skin library (it is generated from
  the installed game per patch and would go stale on the next one), nothing installed into the League client.
  The default party configuration is installed only if none exists, so user changes survive upgrades.
- **Registry**: the "run as administrator" flag is removed from both hives (an older installer wrote HKLM, the
  file's Properties dialog writes HKCU). Start with Windows is an optional task, off by default, writing the
  same HKCU value and quoted path as the tray item, so either side can undo the other; the value is always
  registered for removal because the tray may have turned it on later.
- **Uninstall**: logs, state, the WebView2 profile, built overlays, generated mods and the user-copied injector
  are removed. Skins and custom mods are the user's: an interactive uninstall asks, a silent one keeps them,
  and the data folder is removed only when nothing is left in it. Under an admin uninstall `{localappdata}`
  may resolve to the elevating admin's profile, so on a multi-user machine the desktop user's folder can be
  left untouched; `cargo xtask install-audit uninstalled` checks the single-user case.

## Install and uninstall audit

```powershell
cargo xtask install-audit installed
cargo xtask install-audit uninstalled
cargo xtask install-audit uninstalled --kept-user-content
```

This checks files, tool hashes and registry entries (the uninstall entry, and that no "run as administrator"
flag was left under HKLM or HKCU) against what the installer promises. The install location is read from the
registry, never assumed.
