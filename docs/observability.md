# Observability

## Where logs go

- `%LOCALAPPDATA%\Dekan\logs\dekan.log.YYYY-MM-DD`, rotated daily, with seven days kept.
- Written with `tracing` through a non-blocking appender, so a slow disk never stalls the app. If the queue
  ever has to drop lines, the number of dropped lines is recorded. Nothing is lost silently.
- Each line has a UTC timestamp, the level, the thread, `file:line` and the module.
- The default level is `info`. `DEKAN_LOG=debug` (or a filter such as `dekan_lcu=trace`) adds detail.
  `RUST_LOG` is honored too, but `DEKAN_LOG` wins. An invalid filter falls back to `info` with a warning, so
  it never turns logging off.

## Level policy

| Level | Meaning | Example |
| --- | --- | --- |
| `error` | Trust is broken or an operation was aborted | The game could not be resumed; injection failed |
| `warn` | Something degraded or was refused | An incompatible mod was dropped; a tool is missing; the DLL is close to its build limit |
| `info` | A state change | Phase changed; overlay built; hook confirmed |
| `debug` | Diagnostics | Entries dropped as identical to the game; per-file details |

**Never log inside a loop.** A repeated line in a polling loop is a bug: log the change, not every check.
The default was set to `info` after measuring that most `debug` output was "waiting for the client" noise.

## Events worth looking for

| When | What to find in the log |
| --- | --- |
| Startup | `Dekan starting` (version, elevation, single instance) and the game build |
| Injector support | A warning at startup if the game build is newer than the DLL accepts |
| Skin generated | `Skin bin generated for slot 0`: character, source skin, sizes and checksums of the source and generated bin, links, classification before and after, animation graph, number of changed fields; at `debug`, every changed field |
| Overlay | `Overlay WAD written` per archive: write mode, entries replaced and added, whether the header matches the game's; at `debug`, every changed entry |
| Game reads | `The game opened this archive from the overlay`, one line per archive the injector redirected |
| Match | `Live game data: roster and skins as the game reports them`, `a skin changed during the match`, `Live game event` (kills, multikills, objectives), from the game's local live data API |
| After the match | `Game log: skins the game loaded for this match` and each distinct error from the game's own log |
| User marks | `User marked a problem` with the game time (mm:ss), champion and skin, from `Ctrl+Shift+B` in game or the panel button |
| Screenshots | `Screenshot taken during the match` with the game time of each F12 screenshot the game saved during the match |
| Export | `Match diagnostics exported automatically` when a match ends |
| Injector | `Patcher armed`, `DLL attached to the game`, and any hook failure with its reason |
| Mods | A custom mod refused because of a dangling link |
| Updates | `A newer Dekan release is available` with the current and latest versions; failed checks only at `debug` |

## Byte-level records

Two files hold everything, independent of any particular skin or field:

- `%LOCALAPPDATA%\Dekan\mods\<mod>\META\manifest.json`, for every generated skin: each generated file with its
  size and checksum, the source bin, links, and every field that differs between the game's object and the
  generated one, found by walking every field of every object (nested structures, lists, maps and options
  included), by hash path, with before and after values in hex.
- `%LOCALAPPDATA%\Dekan\overlay_manifest.json`, for the last overlay: per archive, the write mode, the game's and
  the overlay's 272-byte headers in hex, and every replaced or added entry with the game's and the overlay's
  size, type and checksum. It sits outside the folder the game reads.

When a match ends, Dekan zips the last three days of logs, these manifests and the screenshots the game saved
during that match (F12, up to twelve) into the logs folder, keeping the five newest zips. **Export diagnostics** in
the control panel does the same on demand. `Ctrl+Shift+B` marks a moment without leaving the game; it is a
standard Windows shortcut registration, held only while a match is running so other programs keep the keys the
rest of the time, and reads nothing from the game. Other players' names and ids are never written: the match data keeps champion, skin and team only.

Nothing here reads or writes the game's memory: the records come from Dekan's own files, the game's log files
and the game's local live data API (`https://127.0.0.1:2999/liveclientdata/allgamedata`).

The injector host writes its own messages to stderr. Dekan records each one at the level the host gave it:
an `INFO` line from the host is never turned into a warning.

## Planned improvements

- A session id, so one run can be isolated inside a daily file.
- A per-match id on every line of a match.
- A single environment header at the start of each session.
- The local time offset recorded once, to line up with the game's own log, which uses local time.
- Automatic diagnosis that reads Dekan's log and the game's log together and names the likely cause
  (corrupt archive, inconsistent archive, failed hook).
