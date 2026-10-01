# dekan-app

The `dekan.exe` executable. It wires the other crates together, runs the tray application and contains the
logic that decides **what** to inject and **when**.

## Startup

1. **Single instance.** If Dekan is already running, the new process brings it to the front and exits.
2. **User and logging.** It resolves the signed-in desktop user and starts a daily rotating log in
   `%LOCALAPPDATA%\Dekan\logs`. Logging never blocks the app, and any lines dropped under pressure are counted.
3. **Recovery.** If a previous run died while the game was suspended, it resumes the game.
4. **Discovery.** It finds the game install and the injector tools and checks the tools' hashes.
5. **Game build.** It reads the installed game build. After a patch, cached overlays and locale data are thrown
   away. If the injector DLL does not support this build, the user is told right away and not in the middle of
   a match.
6. **Warm-up.** The index of the game's archives is built in the background, so champion select does not have
   to wait for it.
7. **Services.** It starts the supervised tasks: the League client observer, the selection window session, party
   mode and, if enabled, the skin library sync.

## The injection trigger

`trigger.rs` is the heart of the app. It watches the shared state and, during champion select:

1. works out the skin you want, from the selection window and the champion you have locked,
2. waits briefly for the choice to settle before acting (100 ms for the first choice, 900 ms after a change),
   so quickly scrolling through skins does not start a build each time,
3. gathers the mods: the generated skin, your custom mods and your party members' skins (only the ones that
   pass the team check),
4. drops custom mods that no longer fit the current patch,
5. asks `dekan-inject` to build the overlay and arm the injector before the game starts.

The SHA-256 hashes of the audited injector binaries are defined here as well. The packaging tool reads them
from this same source file, so the installer and the running app cannot disagree about which files are
trusted.

## What is inside

| File | Purpose |
| --- | --- |
| `main.rs` | Startup, tray, lifecycle and shutdown |
| `trigger.rs` | Decides when to build and arm, and with which mods; holds the audited tool hashes |
| `overlay_session.rs` | Drives the selection window: sends it the catalog, receives the user's choice |
| `catalog.rs` | Builds the list of skins and chromas for a champion, from the local library or from the client |
| `mods_store.rs` | Custom mod folders, the saved selection and preparing the selected mods |
| `historic_store.rs` | Remembers the last skin used on each champion |
| `party_manager.rs` | Connects party mode to the app state and the tray |
| `skin_sync.rs` | Optional download of a skin library from a GitHub repository the user names in `DEKAN_SKIN_SYNC` (`owner/repo`); there is no built-in source |
| `logging.rs` | Log setup and level handling (`DEKAN_LOG`, `RUST_LOG`) |
| `build.rs` | Embeds the icon, the version details shown in the file properties, and the manifest that keeps Dekan running without administrator rights |

## Logging

The default level is `info`. `error` means something was aborted, `warn` means something was degraded or
refused, `info` records a state change and `debug` holds the details. A line repeated in a polling loop is
treated as a bug: logs record what changed, not every check.

## Running

```powershell
cargo run -p dekan-app --release
$env:DEKAN_LOG = "debug"; cargo run -p dekan-app --release   # with detailed logs
```
