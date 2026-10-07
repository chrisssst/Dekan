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
| `build/mod_compat.rs` | Finds broken references inside a mod before it is used |
| `build/overlay_builder.rs` | Builds the overlay from the installed game and the selected mods; a custom mod's bins get their stale text paths converted to the file references the game now declares before they are merged |
| `build/overlay_cache.rs` | Reuses a previous overlay when the same mods were chosen and the game has not changed |
| `build/overlay.rs` | Overlay configuration and locations |
| `injector/ltk_host.rs` | Protocol spoken with the injector host; checks which game builds the DLL supports |
| `injector/overlay_process.rs` | Starts the host, reads its output without blocking, and kills it if Dekan drops it |
| `pipeline.rs` | Orchestrates build → arm → confirm |
| `injector/trust.rs` | Accepts an injector file only when its Authenticode signature is valid and from League Toolkit's publisher; reads the game-build limit compiled into the DLL |
| `injector/dll_validator.rs` | SHA-256 helpers used to tell installed and published DLLs apart |

## Rules the builder follows

- **Keep the game's bytes and header.** Unchanged entries are copied exactly as the game shipped them, and every
  rebuilt archive keeps the game's header (signature and checksum). The game checks its archives, and a copy
  recompressed or written with another header is rejected as corrupt.
- **A path that several archives hold changes in all of them.** Some paths exist both in a champion archive and
  in a map archive (Zed's shadow is in `Map11.wad.client` too). Changing only one side makes the game report the
  archive as inconsistent, and leaving the path out leaves that part of the skin on its default look, so the
  map archive is rebuilt as well.
- **A game archive is copied once, then reused.** When a mod only replaces entries the archive already has, the
  rebuilt archive is the game file copied byte for byte with the new entries appended and its table updated.
  The copy is kept until the game file changes (outside the served folder when a build does not need it), and
  the map archives a champion shares are copied as soon as the champion is locked in. A 2.5 GB map is copied
  once per patch; switching skins afterwards takes milliseconds.
- **A cached overlay is reused only for the same inputs and the same builder.** Its fingerprint records every
  mod file, base game archive and output archive, plus the builder and its revision; an unknown builder or
  another revision rebuilds it.
- **Every third-party binary is verified first.** If the injector is not signed by League Toolkit's publisher, it is refused. If a
  file is missing, the user is told where it was expected. The injector never falls back to anything silently.

## No suspension

Dekan never suspends the game or opens its threads. The anti-cheat refuses it even from an elevated process
with the debug privilege, so the fallback never worked in a match, and the calls it needed are the ones
antivirus heuristics weigh most. The late path, when the game starts before the injector is armed, builds the
overlay and arms the injector within a fixed time budget instead.

## Testing

```powershell
cargo test -p dekan-inject
```

`tests/native_overlay_faithful.rs` builds an overlay from a real game install and checks, entry by entry, that
unchanged data is byte-identical to the game's and that each archive keeps the game's header
(`DEKAN_FAITHFUL_MODS`, `DEKAN_FAITHFUL_MOD` and `DEKAN_FAITHFUL_SECOND` choose the mods). Anything that affects what happens inside the game still has
to be confirmed in a real match.
