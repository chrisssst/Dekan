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
| Overlay | Archives written, shared entries left untouched, identical entries dropped |
| Injector | `Patcher armed`, `DLL attached to the game`, and any hook failure with its reason |
| Mods | A custom mod refused because of a dangling link |
| Recovery | A suspended game resumed at startup |

The injector host writes its own messages to stderr. Dekan records each one at the level the host gave it:
an `INFO` line from the host is never turned into a warning.

## Planned improvements

- A session id, so one run can be isolated inside a daily file.
- A per-match id on every line of a match.
- A single environment header at the start of each session.
- The local time offset recorded once, to line up with the game's own log, which uses local time.
- Automatic diagnosis that reads Dekan's log and the game's log together and names the likely cause
  (corrupt archive, inconsistent archive, failed hook).
