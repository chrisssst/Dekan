# xtask

Project automation, run through Cargo: `cargo xtask <command>`. Nothing here ships to users.

## Everyday commands

| Command | What it does |
| --- | --- |
| `check` | The error-handling sweep, the comment check, formatting, clippy with warnings as errors and the full test suite |
| `adr008` | Only the error-handling sweep: fails if any discarded `Result` lacks a `// ignore-ok: <reason>` marker |
| `comments` | Fails on any comment in a tracked code file other than a one-line tool directive. `--strip` removes them: each file is read by a lexer for its language (Rust, JS/TS, CSS, HTML, TOML, YAML with the bash or PowerShell inside `run:`, Inno Setup), so text inside strings, raw strings, regular expressions, template literals and heredocs is never touched. A file is written only after it lexes again with the same directives and exactly the same code lines. The removed text goes to `target/comments-removed.md` (`--report <file>`) so it can be moved into the docs |
| `package` | Release build into `dist\`, plus `SHA256SUMS` |
| `installer` | Runs `package`, then Inno Setup, producing `dist\installer\Dekan-Setup-<version>-x64.exe` with the workspace version |
| `install-audit` | After installing or uninstalling, checks files and registry entries against what the installer promises, plus the hash of any injector file the user placed in `tools\` |

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
| `fuzz [iterations] [seed]` | Mutates real game WADs and property files into the parsers; fails on any panic or on a broken round trip of a real skin bin |
| `client-dump <client dir> <path…>` | Prints files of the installed client's game data, such as `v1/champions/99.json` |
| `skin-audit [--root <game>] [--out <file>] [Name…]` | Generates every skin and chroma of every champion from the installed game and reports, per skin: build errors, companion characters and whether each got its `skin0.bin`, skins whose `skin0.bin` dropped a link of the original, generated bins that still reference their source skin's keys, links that point at files the game does not have, and generated paths that a map archive also holds (the overlay builder writes those into the map too). `--keep <dir>` keeps every generated mod for inspection |

`cargo xtask help` prints the full list.

## Release versioning

The installer takes its version from the workspace (`[workspace.package]` in the root `Cargo.toml`). Every
crate inherits that version too, so the installer name, the installer details and the executable details
always agree.
