# dekan-inject

Turns a set of mods into something the game actually loads. It builds the overlay, starts the injector and
makes sure nothing is left behind if something goes wrong.

## Where it sits in the flow

```text
mods chosen ──▶ compatibility check ──▶ overlay build ──▶ injector armed ──▶ game starts ──▶ overlay served
                (drop broken mods)      (native, cached)   (in champ select)                 (DLL in game)
```

1. **Compatibility check.** Every custom mod is checked against the current game. A mod whose data files
   point to files that no longer exist after a patch would crash the loading screen. Such a mod is dropped,
   and the user is warned.
2. **Overlay build.** The builder indexes the installed game's archives. The index is cached and refreshed
   when a file's size or modification time changes. For every archive a mod touches, the builder writes a copy
   that holds the mod's files and the original bytes of everything else. Entries that turn out identical to the
   game's are dropped, so the overlay stays small.
3. **Injector armed.** The injector host (`ltk_patcher_host.exe`) is started during champion select, before
   the game process exists, and receives the overlay location. Dekan sends its commands over stdin and reads
   its status from stdout. The host's own log lines are recorded at the level the host gave them.
4. **Game starts.** The host attaches its DLL to the game. From then on, whenever the game opens one of the
   archives Dekan rebuilt, it reads the overlay copy instead.

## What is inside

| File | Purpose |
| --- | --- |
| `mod_compat.rs` | Finds broken references inside a mod before it is used |
| `overlay_builder.rs` | Builds the overlay from the installed game and the selected mods |
| `overlay_cache.rs` | Reuses a previous overlay when the same mods were chosen and the game has not changed |
| `overlay.rs` | Overlay configuration and locations |
| `ltk_host.rs` | Protocol spoken with the injector host; checks which game builds the DLL supports |
| `overlay_process.rs` | Starts the host, reads its output without blocking, and kills it if Dekan drops it |
| `pipeline.rs` | Orchestrates build → arm → confirm |
| `runner.rs` | Runs external processes without a console window |
| `dll_validator.rs` | Checks a binary's SHA-256 against its audited hash before it is ever run or loaded |
| `suspend.rs` | Game suspension guard and recovery |

## Rules the builder follows

- **Keep the game's bytes.** Unchanged entries are copied exactly as the game shipped them. The game checks
  its archives, and a recompressed copy can be rejected as corrupt.
- **A champion mod never changes a file that a map also contains.** Some paths exist both in a champion archive
  and in a map archive. If the champion's copy is changed but the map's is not, the game detects the mismatch
  and asks for a repair. Those shared entries are therefore left exactly as the game has them, and maps are
  only rewritten by map mods.
- **Every third-party binary is verified first.** If the injector's hash does not match, it is refused. If a
  file is missing, the user is told where it was expected. The injector never falls back to anything silently.

## Suspension and recovery

In the rare fallback where the game has to be paused while the overlay is being prepared, the whole process is
suspended in one call. A guard object resumes it automatically when it goes out of scope, even on a panic. If
Dekan itself dies while the game is suspended, the next start finds the marker file and resumes the game,
after checking that the process id still belongs to the game.

## Testing

```powershell
cargo test -p dekan-inject
```

`tests/native_overlay_faithful.rs` builds an overlay from a real game install and checks, entry by entry, that
unchanged data is byte-identical to the game's. Anything that affects what happens inside the game still has
to be confirmed in a real match.
