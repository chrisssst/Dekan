# dekan-platform

Everything that touches Windows. All Win32 calls live in this crate, so the rest of Dekan can be read and
tested as ordinary Rust.

## Where it sits in the flow

At startup this crate answers the practical questions:

- **Where is the game?** First it looks for a running game process. If none is running, it reads the install
  path from the metadata the Riot Client keeps under `%ProgramData%\Riot Games\Metadata`. Dekan never assumes
  a drive letter, folder name, region or language, so it works wherever the game was installed.
- **Where does Dekan keep its data?** Under the signed-in user's `%LOCALAPPDATA%\Dekan`. The user is resolved
  through the Windows API, not a guessed path.
- **Where are the injector tools?** Only in Dekan's own install folder.
- **Which game build is installed?** It reads the build timestamp from the game executable, so a patch is
  detected and stale caches are thrown away.

During a session it provides the selection window, the tray icon, dialogs and the process control that the
injector needs.

## What is inside

| File | Purpose |
| --- | --- |
| `league/paths.rs` | Finds the game, the data folders and the tools folder |
| `os/system/process.rs` | Finds processes and threads and reads a process's image path |
| `league/game_version.rs` | Reads the game build timestamp and notices when it changes |
| `os/storage/fs.rs` | Atomic writes (write to a temporary file, then rename) and safe archive extraction that rejects path traversal, symlinks and oversized content |
| `ui/pages/overlay_window.rs`, `ui/pages/overlay_model.rs`, `ui/overlay.slint` | The skin selection window that follows the League client window; `overlay_model.rs` holds the search, rows, mods panel and selection logic as plain functions |
| `league/client_settings.rs` | Sets `install.crash_reporting.enabled: false` in the League client's `Config/LeagueClientSettings.yaml`, changing only that line (or adding the missing section) and keeping the file's indentation and line endings; a missing file is left alone |
| `league/client_window.rs` | Locates the League client window so the overlay can follow it |
| `ui/desktop/tray.rs` | The system tray icon; a click opens the control panel, the right-click menu only shows the status, "Open Dekan" and "Quit". Shows the new-version notification; clicking it opens the release page |
| `ui/pages/panel.rs`, `ui/panel.slint` | The control panel: status, options, party, folders and diagnostics in one window |
| `os/storage/preferences.rs` | Persisted on/off preferences: accept matches automatically (off by default) and roll a random skin when none is chosen (on by default), and light match loading (on by default) |
| `ui/pages/welcome.rs`, `ui/pages/party_dialog.rs`, `ui/welcome.slint`, `ui/party.slint`, `ui/desktop/dialog.rs` | The first-run and About window, the party dialog and message boxes |
| `ui/pages/runtime.rs`, `ui/pages/views.rs`, `ui/theme.slint`, `ui/app.slint`, `build.rs` | The `dekan-ui` thread that owns the Slint event loop, the generated window types, the shared design tokens and controls, and the build step that compiles the `.slint` files |
| `ui/locale/i18n.rs` | Translations (Turkish, English) for everything shown outside the League client; falls back to English |
| `os/instance/single_instance.rs`, `os/instance/activation.rs` | Only one Dekan runs at a time; starting it again brings the first one to the front |
| `os/system/autostart.rs` | The "start with Windows" setting |
| `os/system/authenticode.rs` | Verifies a file's Authenticode signature with `WinVerifyTrust` (offline, no revocation fetch) and returns the signer's name |
| `os/system/elevation.rs`, `os/system/user_profile.rs` | Checks the process privileges, runs a program elevated and waits for its exit code (`run_elevated`, a declined prompt is its own outcome), and resolves the real desktop user |
| `ui/desktop/clipboard.rs`, `ui/desktop/shell.rs` | Copying invite codes, opening folders in Explorer, opening `https://` pages (anything else is refused) and message boxes, including a yes/no question |

## Design notes

- **No hardcoded locations.** Every path is discovered at runtime, because Dekan is meant to run on many
  machines with different setups.
- **Text is never hardcoded in one language.** Anything the user reads goes through `ui/locale/i18n.rs`. The selection
  window uses the client's language; dialogs that can open before the client is running use the Windows
  language.
- **The selection window lives outside the client.** Dekan never loads code into the League client. Its window
  follows the client's position and size instead.
- **One interface thread.** Every window lives on the `dekan-ui` thread, which owns the Slint event loop; the
  rest of the app reaches it through `slint::invoke_from_event_loop`. Windows are drawn by Slint's software
  renderer: no GPU work competes with the game and nothing is downloaded at build time.
- **The window trusts nothing it displays.** Skin names are client data shown as plain text; tiles and chroma
  previews arrive as PNG/JPEG bytes Rust fetched and are decoded on the interface thread. The window sends back
  only ids it was given; Rust decides what a choice means and echoes the effective selection.
- **The logic is Rust, the drawing is Slint.** Search (accent folding), rows per column count, the mods panel,
  selection toggling and the empty-state texts are plain functions in `overlay_model.rs` with their own tests.
  A second click on the current pick clears it, which is how "inject nothing" is said and how a restored pick
  is dismissed; a chroma pick lights its whole card.
- **Accessibility is the test surface.** Every control has an accessible role and label. The interface tests
  drive the real windows through it, so the `.slint` files are compiled with resources embedded as files (the
  embedding mode for the software renderer drops the accessible properties) and with debug info in debug
  builds.
- **Language fallback**: the client's exact locale when a dictionary exists, else the same language
  (`tr_TR` → Turkish, `en-*` → English), else Turkish. Champion quotes from unsupported languages are not shown.
- **The overlay window draws no frame and never takes focus on its own.** It is created without activation,
  kept out of the taskbar, and shown or hidden through its window handle with `SW_SHOWNOACTIVATE`/`SW_HIDE`, so
  champion select keeps the keyboard. Clicking the search box takes the foreground (thread-input attach);
  Enter or Escape hands it back to the client. Windows 11 rounds its corners natively; dragging the header and
  the corner grip use the system move and resize loops.
- **Quiet by default**: chroma hover messages and "game or client not running" are debug lines, not state
  changes. A process id with no thread means the process is gone, which the stale-lock checks rely on.
- **A malformed game build record is overwritten** with the current build; refusing to record would hide every
  later patch.
- **An empty dialog is worse than none**: when the welcome or About page cannot be built, the window is
  destroyed.

## Testing

```powershell
cargo test -p dekan-platform
```

The tests that create real windows need a desktop session (or a virtual display when run under Wine).
