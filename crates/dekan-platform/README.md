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
| `paths.rs` | Finds the game, the data folders and the tools folder |
| `process.rs` | Finds processes and threads and reads a process's image path |
| `game_version.rs` | Reads the game build timestamp and notices when it changes |
| `fs.rs` | Atomic writes (write to a temporary file, then rename) and safe archive extraction that rejects path traversal, symlinks and oversized content |
| `overlay_window.rs`, `overlay_ui.html` | The skin selection window: a WebView2 view attached to the League client window |
| `client_window.rs` | Locates the League client window so the overlay can follow it |
| `tray.rs` | The system tray icon; a click opens the control panel, the right-click menu only shows the status, "Open Dekan" and "Quit". Shows the new-version notification; clicking it opens the release page |
| `panel.rs`, `panel_ui.html` | The control panel: status, options, party, folders and diagnostics in one window |
| `preferences.rs` | Persisted on/off preferences: accept matches automatically (off by default) and roll a random skin when none is chosen (on by default) |
| `welcome.rs`, `party_dialog.rs`, `dialog.rs` | The first-run window, the party dialog and message boxes |
| `i18n.rs` | Translations (Turkish, English) for everything shown outside the League client; defaults to Turkish |
| `single_instance.rs`, `activation.rs` | Only one Dekan runs at a time; starting it again brings the first one to the front |
| `autostart.rs` | The "start with Windows" setting |
| `elevation.rs`, `user_profile.rs` | Checks the process privileges and resolves the real desktop user |
| `clipboard.rs`, `shell.rs` | Copying invite codes, opening folders in Explorer and opening `https://` pages (anything else is refused) |

## Design notes

- **No hardcoded locations.** Every path is discovered at runtime, because Dekan is meant to run on many
  machines with different setups.
- **Text is never hardcoded in one language.** Anything the user reads goes through `i18n.rs`. The selection
  window uses the client's language; dialogs that can open before the client is running use the Windows
  language.
- **The selection window lives outside the client.** Dekan never loads code into the League client. Its window
  follows the client's position and size instead.
- **The page trusts nothing it displays.** Everything visible is built with `textContent`, never `innerHTML`
  (skin names are client data), and images are only `data:image/` URIs that Rust fetched, set as `.src`. The
  page sends back only ids it was given; Rust decides what a choice means and echoes the effective selection.
- **One click, no re-render.** A click toggles `.selected` in place, so the entry animation plays only on the
  first paint of a champion's catalog; a chroma pick lights its whole card like the client's picker. A second
  click on the current pick clears it, which is how "inject nothing" is said and how a restored pick is
  dismissed.
- **Page details that must stay in step with Rust**: the `hidden` attribute always wins over a class that sets
  `display` (otherwise the placeholder and the list both show and push the list below the fold), and the CSS
  corner radius equals `OVERLAY_CORNER_RADIUS`, the window region `SetWindowRgn` clips to. The resize grip is
  inset from the corner because the rounded region clips the corner pixels.
- **Language fallback**: Turkish (`tr_*`) and English (`en_*`) are the only supported UI languages.
  Unsupported client locales fall back to Turkish. The application starts in Turkish by default.
- **The overlay window draws no frame.** `WS_THICKFRAME` only enables the native resize loop the grip starts,
  and `WM_NCCALCSIZE` hands the whole window to the client area. The window takes the foreground; keyboard
  focus goes to the WebView, never to the host.
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
