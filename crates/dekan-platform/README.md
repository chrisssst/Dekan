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
| `os/system/launch/process.rs` | Finds processes and threads and reads a process's image path |
| `league/game_version.rs` | Reads the game build timestamp and notices when it changes |
| `os/storage/fs/mod.rs`, `os/storage/fs/archive.rs` | Atomic writes (write to a temporary file, then rename) and tree mirroring; `fs/archive.rs` is the safe archive extraction that rejects path traversal, symlinks and oversized content |
| `ui/pages/overlay_window/mod.rs`, `ui/pages/overlay_model/mod.rs`, `ui/overlay/overlay.slint` | The skin selection window that follows the League client window; `overlay_model/mod.rs` holds the search, rows, mods panel and selection logic as plain functions |
| `ui/pages/overlay_window/view.rs`, `ui/pages/overlay_window/render.rs`, `ui/pages/overlay_window/placement.rs` | Creating and wiring the Slint view, rendering the model into it, and placing the window beside the client |
| `ui/overlay/parts/overlay_types.slint`, `ui/overlay/parts/overlay_cards.slint`, `ui/overlay/parts/overlay_controls.slint`, `ui/overlay/overlay_header.slint`, `ui/overlay/overlay_list.slint`, `ui/overlay/overlay_preview.slint` | The overlay's data types, skin cards and gems, buttons and tabs, header, scrolling list and chroma preview |
| `league/client_settings.rs` | Sets `install.crash_reporting.enabled: false` in the League client's `Config/LeagueClientSettings.yaml`, changing only that line (or adding the missing section) and keeping the file's indentation and line endings; a missing file is left alone |
| `league/client_window.rs` | Locates the League client window so the overlay can follow it |
| `ui/desktop/tray/mod.rs`, `ui/desktop/tray/window.rs` | The system tray icon (`tray/window.rs` is its Win32 window and message loop); a click opens the control panel, the right-click menu only shows the status, "Open Dekan" and "Quit". Shows the new-version notification; clicking it opens the release page |
| `ui/pages/panel/mod.rs`, `ui/panel/panel.slint`, `ui/panel/panel_types.slint`, `ui/panel/panel_parts.slint` | The control panel: status, options, party, folders and diagnostics in one window |
| `os/storage/preferences.rs` | Persisted on/off preferences: skin injection (on by default), accept matches automatically (off by default), roll a random skin when none is chosen (on by default) and light match loading (on by default) |
| `os/storage/disk.rs` | Whether the disk under a path is solid-state or rotational (seek-penalty query on its volume), logged at startup so performance reports show what the game is read from |
| `ui/pages/welcome/mod.rs`, `ui/welcome.slint` | The first-run and About window |
| `ui/pages/party_dialog/mod.rs`, `ui/party.slint` | The party windows: the created-code and join dialogs (`PartyDialog`) and the lobby invite (`PartyInvite`), a Join / Later question shown when friends in the same League lobby run Dekan and closed again if the lobby changes before it is answered |
| `ui/desktop/dialog.rs` | The native file picker |
| `ui/desktop/hotkey.rs` | The `Ctrl+Shift+B` shortcut that sends the tray's "mark a problem" event; it is registered only while a match is being played |
| `os/system/identity/version.rs` | The version shown to the user, with two numbers (`1.2`) |
| `ui/pages/runtime.rs`, `ui/pages/views.rs`, `ui/theme.slint`, `ui/app.slint`, `build.rs` | The `dekan-ui` thread that owns the Slint event loop, the generated window types, the shared design tokens and controls, and the build step that compiles the `.slint` files |
| `ui/locale/i18n/mod.rs`, `ui/locale/i18n/tr.rs`, `ui/locale/i18n/en.rs` | Translations (Turkish and English, one table per file) for everything shown outside the League client; falls back to Turkish |
| `os/instance/single_instance.rs`, `os/instance/activation.rs` | Only one Dekan runs at a time; starting it again brings the first one to the front |
| `os/system/launch/autostart.rs` | The "start with Windows" setting |
| `os/system/authenticode/mod.rs` | Verifies a file's Authenticode signature with `WinVerifyTrust` (offline, no revocation fetch) and returns the signer's name |
| `os/system/launch/elevation.rs`, `os/system/identity/user_profile.rs` | Checks the process privileges, runs a program elevated and waits for its exit code (`run_elevated`, a declined prompt is its own outcome), and resolves the real desktop user |
| `ui/desktop/clipboard.rs`, `ui/desktop/shell.rs` | Copying invite codes, opening folders in Explorer, opening `https://` pages (anything else is refused) and message boxes, including a yes/no question |

## Design notes

- **No hardcoded locations.** Every path is discovered at runtime, because Dekan is meant to run on many
  machines with different setups.
- **Text is never hardcoded in one language.** Anything the user reads goes through `ui/locale/i18n/mod.rs`. The selection
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
  selection toggling and the empty-state texts are plain functions in `overlay_model/mod.rs` with their own tests.
  A second click on the current pick clears it, which is how "inject nothing" is said and how a restored pick
  is dismissed; a chroma pick lights its whole card.
- **Accessibility is the test surface.** Every control has an accessible role and label. The interface tests
  drive the real windows through it, so the `.slint` files are compiled with resources embedded as files (the
  embedding mode for the software renderer drops the accessible properties) and with debug info in debug
  builds.
- **Language fallback**: Turkish is the default. Turkish and English client locales select the matching
  dictionary; unsupported language families fall back to Turkish. Champion quote text follows those two
  supported dictionaries.
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
