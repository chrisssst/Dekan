# xtask

Project automation, run through Cargo: `cargo xtask <command>`. Nothing here ships to users.

## Everyday commands

| Command | What it does |
| --- | --- |
| `check` | The error-handling sweep, formatting, clippy with warnings as errors and the full test suite |
| `adr008` | Only the error-handling sweep: fails if any discarded `Result` lacks a `// ignore-ok: <reason>` comment |
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

`cargo xtask help` prints the full list.

## Release versioning

The installer takes its version from the workspace (`[workspace.package]` in the root `Cargo.toml`). Every
crate inherits that version too, so the installer name, the installer details and the executable details
always agree.
