# dekan-app

The `dekan.exe` executable. It wires the other crates together, runs the tray application and contains the
logic that decides **what** to inject and **when**.

## Startup

1. **Single instance.** If Dekan is already running, the new process brings it to the front and exits.
2. **User and logging.** It resolves the signed-in desktop user and starts a daily rotating log in
   `%LOCALAPPDATA%\Dekan\logs`. Logging never blocks the app, and any lines dropped under pressure are counted.
3. **Discovery.** It finds the game install and the injector tools and checks the tools' hashes.
4. **Game build.** It reads the installed game build. After a patch, cached overlays and locale data are thrown
   away. If the injector DLL does not support this build, the user is told right away and not in the middle of
   a match.
5. **Warm-up.** The index of the game's archives is built in the background, so champion select does not have
   to wait for it.
6. **Services.** It starts the supervised tasks: the League client observer, the selection window session, party
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
| `live_game.rs` | During a match, reads the game's local live data API every five seconds (roster skins, skin changes, events) and, after the match, the game's own log (skins loaded, errors); only reads, never touches the game |
| `update_check.rs` | Reads the latest published release from GitHub every six hours and announces a newer one once (tray notification, control panel line); never downloads or runs anything. Off with `DEKAN_UPDATE_CHECK=0` |
| `logging.rs` | Log setup and level handling (`DEKAN_LOG`, `RUST_LOG`) |
| `build.rs` | Embeds the icon, the version details shown in the file properties, and the manifest that keeps Dekan running without administrator rights |

## Design notes

- **Without the injector nothing is built, and the base skin is not registered in the client**: registering it
  would take the player's own skin away for nothing. Missing tools are reported once per match, and a client
  skin that diverges from the registered one is answered once per value the player causes, never once per tick.
- **The catalog never waits on the client.** The client lookup is best effort: a silent client costs names,
  never the catalog. For Rift Classic, what can be offered is decided by the installed game's `jade_*` tree,
  not by the library. A chroma preview's asset path stays on the Rust side; the page only learns that a
  preview exists and asks for it by id.
- **Chroma previews never hold the selection.** When a champion's catalog is sent, every preview it offers
  is fetched from the client with one connection, four at a time, and pushed to the page as it arrives.
  The session loop only polls that stream, so a `Select` is handled at once even while previews are still
  coming; hovering a chroma whose preview is not there yet waits for the stream instead of fetching it again.
- **Auto-accept waits a moment** after the ready check appears: the client refuses an accept sent on the very
  first phase event about as often as it takes it. If the user accepted, declined or dodged meanwhile, nothing
  is sent.
- **Party mode does not start without randomness**: when the operating system's random generator fails,
  nothing secret can be made.
- **The game build is read from the executable's PE headers only** (a few hundred bytes) and compared with the
  last run; an unknown game folder is not an error.
- **`build.rs` fails the build** when the resources cannot be embedded: an executable without the icon, the
  version details (shown with two numbers, `1.2`) and the `asInvoker` manifest must never ship.

## Logging

The default level is `info`. `error` means something was aborted, `warn` means something was degraded or
refused, `info` records a state change and `debug` holds the details. A line repeated in a polling loop is
treated as a bug: logs record what changed, not every check.

## Running

```powershell
cargo run -p dekan-app --release
$env:DEKAN_LOG = "debug"; cargo run -p dekan-app --release   # with detailed logs
```

## Testing

```powershell
cargo test -p dekan-app --test skin_pipeline
```

`tests/skin_pipeline.rs` builds a small, deterministic game install (Zed with a legendary skin, its chroma, the
shadow companion, animation graphs, a texture, a Summoner's Rift map that shares the shadow and a TFT map) and a
`.fantome` skin mod, then runs them through the production code: the skin generator, mod import, catalog,
staging, the compatibility check and the overlay builder. It then opens the archives the game would mount (the
overlay file where one exists, the game's otherwise, as the injector redirects them) and checks the result.
No League client, login, match or network is needed. It checks files, not rendering: how the skin looks in game
is only visible in a real match (see `docs/build-and-ci.md`, "Test layers").
