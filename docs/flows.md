# End-to-end flows

## 1. Store skin: from champion select to the match

```text
Client enters champion select
  └─ dekan-lcu publishes the phase and the team roster
     └─ the selection window opens next to the client; the player picks a skin
        └─ the trigger settles the choice (100 ms for the first pick, 900 ms after a change)
           └─ the champion and selection are read again from the client
              └─ dekan-classic opens DATA/FINAL/Champions/<Name>.wad.client and turns the chosen skin
                 into the default one (SkinN → Skin0), companions included
              └─ dekan-inject builds the overlay
                 · entries identical to the game's are dropped
                 · paths shared with map archives are left as the game has them
           └─ the injector host is armed while champion select is still running
           └─ the skin is registered with the client (an owned skin as itself, an unowned one as the default)
Game starts
  └─ the injector DLL attaches and confirms its hook
     └─ the game opens the rebuilt archives from the overlay; the skin loads
```

**Why the loading screen shows the default name for an unowned skin.** The name on the loading card comes
from the skin id the server received. The client refuses to register a skin you do not own, so the card
shows the champion's default name while the skin's art still loads from the overlay. Showing the skin's name
would require writing to game memory, which Dekan deliberately does not do.

## 2. Custom mods

```text
The player drops a .fantome into %LOCALAPPDATA%\Dekan\custom_mods\<category>
  └─ the mod appears in the Mods tab and is selected there
     └─ the selection joins the other mods for the next build
        └─ compatibility check: every data file the mod links to must exist in the game or in the mod
           · a dangling link means the mod was made for an older patch → it is dropped with a warning
             (it would otherwise crash the loading screen)
        └─ the overlay builder merges what is left, with the same rules as for skins
```

Large mods that touch many game archives (a map overhaul can touch over a hundred) are expensive to build
inside champion select. Dropping entries identical to the game's keeps most of them manageable. Showing the
build cost when the mod is selected is still open work.

## 3. Classic Rift

```text
A classic champion is detected (champion ids offset by 60000, skin ids by 60000000)
  └─ dekan-classic builds the mod from the installed game
     · classic characters are matched by name in the archive's table of contents
     · without a hash table, the champion's data files are scanned to find them
```

The classic ids are kept as they are. They are never forced back to the regular champion.

## 4. Party mode

```text
A player creates or joins a room from the tray
  └─ dekan-party connects to the relay with a room id derived from the invite key
     └─ each member announces champion and skin, encrypted on their own machine
        └─ the trigger keeps only announcements whose champion matches the real roster
           reported by the client for that player's slot
           └─ each teammate's skin is generated like your own and merged into the same overlay
              └─ you see your friends' skins in the match
```

Open work: the selection window does not yet show who is in the room or which skins were applied, and the
mode has not been proven with several players in one real match.

## 5. Startup and patches

```text
Dekan starts → finds the game → reads the game build
  · new build? cached overlays and locale data are discarded; the archive index revalidates
    itself by file size and modification time
  · build newer than the injector DLL supports? the user is warned at startup
  └─ the client observer connects whenever the client opens; with no local skin library,
     the catalog is filled from the client's own skin list
```

## 6. Shutdown and recovery

- When the game has to be suspended (a rare fallback), a guard resumes it on `Drop`, even during a panic.
- If Dekan dies while the game is suspended, a marker file records the process. On the next start, Dekan
  resumes it after checking that the process id still belongs to the game.
- Uninstalling removes logs, state, overlays and generated mods, and asks before deleting the user's own
  skins. `cargo xtask install-audit` checks the result.
