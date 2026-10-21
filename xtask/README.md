# xtask

Project automation, run through Cargo: `cargo xtask <command>`. Nothing here ships to users.

## Everyday commands

| Command | What it does |
| --- | --- |
| `check` | The error-handling sweep, the comment check, the file size check (no code file over 400 lines, no folder over 5 code files, tests included), formatting, clippy with warnings as errors and the full test suite |
| `adr008` | Only the error-handling sweep: fails if any discarded `Result` lacks a `// ignore-ok: <reason>` marker |
| `comments` | Fails on any comment in a code file that git tracks or that is untracked and not ignored, other than a one-line tool directive. `--strip` removes them: each file is read by a lexer for its language (Rust, JS/TS, CSS, HTML, TOML, YAML with the bash or PowerShell inside `run:`, Inno Setup), so text inside strings, raw strings, regular expressions, template literals and heredocs is never touched. A file is written only after it lexes again with the same directives and exactly the same code lines. The removed text goes to `target/comments-removed.md` (`--report <file>`) so it can be moved into the docs |
| `package` | Locked release build into `dist\`: `dekan.exe`, `dekan-relay.exe` and a default `party.json`, plus `SHA256SUMS`. Fails when the relay binary was not built, so a package never ships without it |
| `installer` | Runs `package`, then Inno Setup, producing `dist\installer\Dekan-Setup-<version>-x64.exe` with the workspace version. `--prebuilt` packages the `dist\dekan.exe` already there, so the release can sign it first |
| `set-version <major.minor>` | Stamps `major.minor.0` into the workspace `Cargo.toml` and `Cargo.lock`. If writing the lock fails, `Cargo.toml` is restored. The release workflow runs it; nobody runs it by hand |
| `release-key` | Generates the update signing key pair: the private key goes, as hex, to a new file that must be outside the repository; the public key is printed as the `RELEASE_PUBLIC_KEY` line for `dekan-app` |
| `sign-sums` | Signs a `SHA256SUMS` with `RELEASE_SIGNING_KEY` into `SHA256SUMS.sig` next to it, after checking the key matches the embedded public key |
| `install-audit installed` or `uninstalled [--kept-user-content]` | After installing or uninstalling, checks files and registry entries against what the installer promises, plus the publisher signature of any injector file the user placed in `tools\` |

## Diagnostic probes

These commands run the real code against the game installed on the machine. Use them when you need proof
rather than an assumption.

| Command | What it proves |
| --- | --- |
| `wad-probe <filter…>` | Every matching entry in the game archives decodes |
| `wad-types` | How many entries of each storage type the installed game uses |
| `wad-writer-probe` | Copying real archives through the writer reproduces every entry |
| `classic-probe <Name…>` | Classic Rift mods can be built from the installed game |
| `client-probe` | Where the League client window is and where the selection window would sit |
| `overlay-demo` | Shows the selection window attached to the client for 30 seconds |
| `ipc-probe` | A scripted click in the real selection window reaches the Rust side |
| `library-probe`, `catalog-demo` | What the skin catalog contains for a champion |
| `client-audit [--client <dir>] [--root <game>] [--out <file>]` | Reads the skin data from the installed client, builds Dekan's catalog with Dekan's own code and reports entries not offered, wrong parent skins, entries without a skin file, champions Dekan cannot name without the client running, and for Rift Classic the entries offered versus listed, with every offered Classic mod generated |
| `harness [--root <game>] [--out <file>]` | For every champion (latest skin and one chroma), the reported cases and Rift Classic: generates the skin, builds the real overlay, and checks that every overlay entry decodes, unchanged entries keep the game's exact bytes and no changed path disagrees with another mounted archive |
| `smoke <dekan.exe> <tools dir>` | Runs a release binary without tools, with wrong tools and with the audited injector; checks the real log (refusal, every supervised task up, no error) and that a second launch opens the first one's control panel and exits. The first two scenarios are skipped when `%ProgramFiles%/Dekan/tools` holds an injector, because discovery prefers it |
| `mesh-roundtrip [--root <game>]` | Every skinned mesh (`.skn` v4) of the game is read and written back byte for byte, and every skeleton (`.skl`) to the same size and structure (its padding bytes hold exporter leftovers and are written as zero); also prints the largest joint and influence counts the game ships |
| `prop-roundtrip [--root <game>]` | Every object of every champion and map bin is read into the property tree and written back; fails on any byte that differs |
| `fuzz [iterations] [seed]` | Mutates real game WADs and property files into the parsers; fails on any panic or on a broken round trip of a real skin bin |
| `client-dump <client dir> <path…>` | Prints files of the installed client's game data, such as `v1/champions/99.json` |
| `skin-audit [--root <game>] [--out <file>] [Name…]` | Generates every skin and chroma of every champion from the installed game and reports, per skin: build errors, companion characters and whether each got its `skin0.bin`, skins whose `skin0.bin` dropped a link of the original, generated bins that still reference their source skin's keys, links that point at files the game does not have, and generated paths that a map archive also holds (the overlay builder writes those into the map too). `--keep <dir>` keeps every generated mod for inspection |
| `mod-audit [--root <game>] <file|folder>…` | Copies custom mods to a scratch folder, runs the startup repair on the copies and checks each one: dangling links before and after, that only the link list of a relinked bin changed and every other entry is bit-identical to the original, which entries equal the game's, object references the game's own bin does not also leave unresolved, and referenced files the game and the mod lack. Ends with the corpus total: how many mods show each class of break, most frequent first. Exits non-zero when a link stays dangling |

The audits and probes exit with status 1 when they find a defect or cannot run, and `harness` also when it has no
cases to check. A command started without its required arguments (`mod-audit` without mods, `smoke` without a
tools folder, `install-audit` without a phase) prints its usage and exits with status 2.

`cargo xtask help` prints the full list.

## Release versioning

The installer takes its version from the workspace (`[workspace.package]` in the root `Cargo.toml`). Every
crate inherits that version too, so the installer name, the installer details and the executable details
always agree. `cargo xtask set-version` is how the release workflow stamps the version it picked; `Cargo.toml`'s
own value is only the floor for it.
