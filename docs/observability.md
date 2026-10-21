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
| `error` | Trust is broken or an operation was aborted | The overlay could not be built; the patcher could not be started; a supervised task panicked |
| `warn` | Something degraded or was refused | An incompatible mod was dropped; a tool is missing; the DLL is close to its build limit |
| `info` | A state change | Phase changed; overlay built; hook confirmed |
| `debug` | Diagnostics | Entries dropped as identical to the game; per-file details |

**Never log inside a loop.** A repeated line in a polling loop is a bug: log the change, not every check.
The default was set to `info` after measuring that most `debug` output was "waiting for the client" noise.

## Events worth looking for

| When | What to find in the log |
| --- | --- |
| Startup | `Dekan starting` (version, elevation, single instance), the game build, `League install found` with `found_by` (the running game or client executable, or `riot_client_installs`), `Game WAD index ready` (wads, elapsed_ms, and `game_disk`/`state_disk`: `ssd`, `hdd` or `unknown`), `Companion characters indexed` (elapsed_s) and `Removed generated skins that could no longer be reused` (removed) |
| Injector support | At startup, an `error` (`The installed game build is newer than the patcher DLL accepts`) when the game build is newer than the DLL accepts, a `warn` when the DLL accepts it only until the next patch (`days_left`) or its limit could not be read; `Dekan stopped at startup: the injector files are missing from the tools folder` or `Dekan stopped at startup: an injector file is not signed by its publisher` when the injector is unusable; `Injector file found` for the host and the DLL with size, modification time and SHA-256, so a report shows which injector build the user has; `Injector files verified by their publisher signature` once per arming |
| Injector failures | `The patcher DLL reached the game after it had started loading and stood down` when the hook arrived mid-load; patcher exit codes carry `exit_code_hex` (NTSTATUS form, e.g. `0xC0000005`) and `closed_by_another_program`, true for `0x40010004`, the code a process gets when something else terminates it (an antivirus, typically); the tray status then says so instead of a generic failure; `The patcher exited before confirming the hook; the overlay is not active`, `Hook not confirmed` and `Late-path budget already spent; not waiting for the hook` (`warn`); `Injection failed; the skin will not load` and `Patcher DLL reached end of life; a refreshed DLL is required for this game patch` (`error`, with the host's reason); `Arming aborted: an injector file is missing or not signed by its publisher`, `Arming aborted: the overlay could not be built` and `Arming aborted: the patcher could not be started` (`error`); `Native overlay build failed` (`warn`) |
| Injection off | `Skin injection setting loaded` at startup and `Skin injection changed from the control panel` with `enabled`; while off, `Skin injection is turned off; this pick will not be injected` once per champion select and `Skin injection turned off; stopping the running patcher` when a match was already patched |
| Mode and lobby | `Lobby queue` with `queue_id`, `game_mode`, `map_id` and whether champions are picked in the lobby; `Champions are picked in the lobby` with the champions; `Skins registered in the lobby slots`; at game start `Injecting the skin picked in the lobby for the champion this match gave` |
| Saved skins | `Saved skin restored as the injection target` with `origin` (Preset or Historic) and the active `profile`; `Saved skin restored for the lobby's other champion`; a saved skin is not restored while a custom skin is selected for the champion, and `A custom skin was selected; the saved skin restored for this champion is dropped` when one is picked after the restore; `Skin preset changed` (pinned or not); `Skin profile switched`, `created`, `deleted`; `Skin presets restored` at startup |
| Chroma previews | `Skin catalog sent to the overlay` counts `chromas` and `chromas_with_preview` (those the client gave an image path for). `Chroma previews fetched from the client` closes the prefetch (asked, fetched, bytes, elapsed_ms); a warning with `failed` and `first_error` when some did not arrive, and one when an image could not be decoded. At `debug`: the ids without a preview, each image fetched or refused with its reason, each hover answered from the cache or while the prefetch runs, and each decoded image's size |
| Skin generated | `Skin bin generated for slot 0`: character, source skin, sizes and checksums of the source and generated bin, links, classification before and after, animation graph, number of changed fields; at `debug`, every changed field |
| Overlay | `Native overlay built` once per build: archives, rewritten, bytes, strays removed and `stuck` (strays that could not be removed; the cache is then not recorded), `header_mismatch` (archives whose header differs from the game's). At `debug`, `Overlay WAD written` per archive (write mode, entries replaced and added, header match) and every changed entry; `overlay_manifest.json` keeps the same detail |
| Game reads | `The game opened this archive from the overlay`, one line per archive the injector redirected |
| Match | `Live game data: roster and skins as the game reports them`, `a skin changed during the match`, `Live game event` (kills, multikills, objectives), from the game's local live data API |
| After the match | `Game log: skins the game loaded for this match` and each distinct error from the game's own log, also after a crash that leaves the client on the reconnect screen |
| Party | Lobby: `Lobby party changed` (member count, at `debug`; the party id is never logged), `Party: joined the room of the lobby` (`share`: true when this lobby was accepted before), `Party: friends in the lobby run Dekan; asking to share picks`, then `Party: sharing picks with the friends in the lobby` or `Party: the lobby invite was declined; the lobby room closes`; `Party: the lobby invite closed unanswered; the queue started or the lobby changed`; `Party: the lobby changed; its room closes`; `Party: no relay configured; the lobby room is not opened`. Room: `Party: joining the room`, `Party: connected to the room`, `Party: room picks changed` (members, announcing, picks), `Party: left the room`; `Party: lost the relay; reconnecting` once per run of failures (repeats at `debug`: `Party: the relay is still unreachable`); `Party: the relay refused us because the room is full; not retrying`; `Party: a member's announcement was refused` once per member and reason; `Party: could not seal our pick for the room` (`error`). The client drops a session that hears nothing for two ping intervals (`the relay stopped answering`), reads close code 4000 as another connection of the same member replacing this one and 4009 as a full room. `Party: connected to the room` is `info` on the first connection and after a warned failure, `debug` on quick reconnects |
| Relay | Self-hosted relay: `Relay: room opened` / `Relay: room closed` (room count, never the room id or an IP), `Relay: an older connection of the same member was replaced` (close 4000), `Join refused: the room is full` (at most one `warn` a minute with the `suppressed` count; close 4009 or HTTP 409), `Session closed: message over the size limit`, `Snapshot skipped for members with a full queue`, `Connection refused: the relay is at its connection limit` and `Accept error; retrying until it clears` once until they clear. Cloudflare worker (Workers Logs, JSON lines without keys, blobs or IPs): `join_refused_room_full`, `join_refused_invalid_key`, `member_replaced`, `join_dropped`, `skin_dropped`, `message_dropped`, `broadcast_send_failed`, `socket_error`, `forget_failed`, `replace_close_failed`, `request_refused`, `room_unavailable` |
| Reconnect | `Game left for the reconnect screen; the skin can be changed before reconnecting` (the skin kept), then the usual rebuild lines if the skin changes; `Reconnect build abandoned: the match ended` |
| User marks | `User marked a problem` with the game time (mm:ss), champion and skin, from `Ctrl+Shift+B` in game or the panel button |
| Screenshots | `Screenshot taken during the match` with the game time of each F12 screenshot the game saved during the match |
| Export | `Match diagnostics exported automatically` when a match ends |
| Injector | `Patcher armed`, `DLL attached to the game`, and any hook failure with its reason |
| Mods | A custom mod with asset formats the game no longer uses: `Custom mod carries asset formats the installed game no longer uses` (formats such as `skn v65537`); a `.modpkg` that needs repairs: `Custom mod package needs repairs for the installed patch`; `Custom mods restored to their original files at the user's request` (mods, files) from the panel. At startup `Custom mods checked against the installed patch` (checked, unchanged, repaired, incompatible, elapsed_ms), one `Custom mod repaired for the installed patch` per rewritten mod with its backup folder; `Custom mod relinked to the files of the installed patch` when a prepared copy is fixed at build time; a mod that links files the game no longer has is refused with `Custom mod links files the installed game no longer has and has no single successor` at startup and `Custom mod is incompatible with the installed patch` at build time (`dangling` lists the links) |
| Overlay cache | `Overlay cache fingerprint not recorded`, `Overlay cache not recorded: stray WADs from an earlier build are still in the overlay` and `Overlay cache fingerprint could not be removed` (`warn`: the next match rebuilds, or a stale overlay may be reused); `Game WAD index not saved` and the bin-scan and Classic character cache warnings (`Bin-scan cache not written; the next build scans again`). At `debug`, why a cached overlay was not reused: `Overlay cache: mod list changed`, `schema version mismatch`, `fingerprint unreadable` |
| Client connection | `League Client lockfile detected. Initiating connection.` and `LCU session closed; match state reset` (`info`; the observer then waits with a backoff and `League Client not running; waiting for it`); `LCU session ended or encountered error` and `LCU WebSocket read error` (`warn`); `Could not sync the initial gameflow phase`, `Could not read the lobby on connect`, `Could not read the champ select session on connect`; `LCU event frame could not be decoded; the client protocol may have changed` and `Unmapped LCU gameflow phase; keeping the current phase` (the client schema changed); `The client refused the lobby slot skins`; `LCU answered with a non-success status` |
| Supervision and crashes | `Spawned supervised task` (task, essential); `Essential supervised task ended before shutdown; its work has stopped` (`error`) or the same line without "Essential" (`warn`) when a task returns early; `Supervised task panicked` (`error`, with the panic message); at shutdown `Supervised task did not stop when cancelled; aborted` after a 5 s grace. `A thread panicked` (`error`, with thread, location and message) comes from the panic hook. `Log writer queue overflowed; lines were dropped` every 30 s while lines are lost, and `Log lines were dropped this session because the writer queue was full` at exit |
| Updates | `A newer Dekan release is available` with the current and latest versions; failed checks only at `debug`. The answer: `The user accepted the update` / `The user declined the update`; `Update verified; closing Dekan to install it`, `Starting the installer; it closes this process and opens the new version`; failures `The update was not downloaded or failed verification` (with the reason; `error` and `The update failed verification and was discarded` when the signature or checksum is wrong), `The downloaded installer no longer matches the signed release; it is not run`, `The installer could not be started; Dekan reopens` and `The installer stopped before updating, or Windows permission was declined` (with the exit code). `This copy was not installed by the setup; only the download page is offered` for portable copies |

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
the control panel does the same on demand. Logs and manifests are redacted on the way into the zip: the Windows
profile folder becomes `%USERPROFILE%` (so the account name does not leave the machine) and every UUID, such as
the player's PUUID, becomes `<uuid>`; screenshots are copied as they are. `Ctrl+Shift+B` marks a moment without leaving the game; it is a
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
