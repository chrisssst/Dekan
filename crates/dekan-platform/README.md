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
| `process.rs` | Finds processes and threads; suspends and resumes a whole process in one call |
| `game_version.rs` | Reads the game build timestamp and notices when it changes |
| `fs.rs` | Atomic writes (write to a temporary file, then rename) and safe archive extraction that rejects path traversal, symlinks and oversized content |
| `overlay_window.rs`, `overlay_ui.html` | The skin selection window: a WebView2 view attached to the League client window |
| `client_window.rs` | Locates the League client window so the overlay can follow it |
| `tray.rs` | The system tray icon and menu |
| `welcome.rs`, `party_dialog.rs`, `dialog.rs` | The first-run window, the party dialog and message boxes |
| `i18n.rs` | Translations (English, Portuguese, Spanish) for everything shown outside the League client; falls back to English |
| `single_instance.rs`, `activation.rs` | Only one Dekan runs at a time; starting it again brings the first one to the front |
| `autostart.rs` | The "start with Windows" setting |
| `elevation.rs`, `user_profile.rs` | Checks the process privileges and resolves the real desktop user |
| `clipboard.rs`, `shell.rs` | Copying invite codes and opening folders in Explorer |

## Design notes

- **No hardcoded locations.** Every path is discovered at runtime, because Dekan is meant to run on many
  machines with different setups.
- **Text is never hardcoded in one language.** Anything the user reads goes through `i18n.rs`. The selection
  window uses the client's language; dialogs that can open before the client is running use the Windows
  language.
- **The selection window lives outside the client.** Dekan never loads code into the League client. Its window
  follows the client's position and size instead.

## Testing

```powershell
cargo test -p dekan-platform
```

The tests that create real windows need a desktop session (or a virtual display when run under Wine).
