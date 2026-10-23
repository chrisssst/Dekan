# End-to-end flows

## 1. Store skin: from champion select to the match

```text
Client enters champion select
  └─ dekan-lcu publishes the phase and the team roster
     └─ the selection window opens next to the client; the player picks a skin
        · when nothing else is chosen, the champion's preset in the active profile is restored, or else
          the last skin used on it
        · when the champion locks in (finalization) with no skin chosen, no skin picked in the client and
          no explicit "Clear", a random skin is rolled (control panel option, on by default)
        └─ the trigger settles the choice (100 ms for the first pick, 900 ms after a change)
           └─ the champion and selection are read again from the client
              └─ dekan-classic opens DATA/FINAL/Champions/<Name>.wad.client and turns the chosen skin
                 into the default one (SkinN → Skin0), companions included (found by scanning the
                 champion's property files; a chroma without its own companion file uses its base skin's)
                 · a skin with several forms gets the Ctrl+5 form cycle in its own animation graph
                 · a spell clip the skin only has as its own variants gets the default name back
                 · a skin already generated for the same archive, Dekan build and options is reused as is,
                   and a second request for the same skin waits for the one in progress
              └─ dekan-inject builds the overlay
                 · entries identical to the game's are dropped
                 · paths shared with map archives change in the map archive too
                 · a game archive is copied once per patch and reused (maps are copied when the
                   champion is locked in)
           └─ the injector host is armed while champion select is still running
           └─ the skin is registered with the client (an owned skin as itself, an unowned one as the default)
Game starts
  └─ the injector DLL attaches and confirms its hook
     └─ the game opens the rebuilt archives from the overlay; the skin loads
```

**Why Ctrl+5 works through the animation graph.** `Ctrl+5` makes the game play the champion's `Toggle` clip.
A skin with forms normally switches them through the gear the server tracks for its owner; under the default
skin's id there is no gear to switch, so the generated graph cycles the forms itself by which form part is
visible. The chosen form is kept in an invisible marker part that only `Ctrl+5` changes, held on a track of its
own by a looping clip that moves no bone, and every animation the game plays by name (walking, spells, recall)
enters a complete copy of the graph for the marked form, so the form stays until the next `Ctrl+5`. Ability and
idle effects, materials and gear-picked animations follow the form too; a form that needs another mesh, and
states a skin's server script turns on, are listed by `cargo xtask skin-audit` instead. Only what you see on your own screen changes.

**Why some skins lose a special behaviour.** The game runs a script of its own for some skins, keyed by the
skin id the server received (for example a skin that changes its music or reacts to the match). The generated
skin loads under the default id, so those scripts do not start. They are game logic and are left untouched.

**Why a game that is already loading is never hooked.** When the patcher is ready only after the game process has
been running for more than two seconds, the game is already reading its archives. Hooking it then mixes files from
the game and from the overlay, and the game can crash. The patcher waits for that process to close and hooks the
next one (a reconnect) instead; the late path leaves the match with the default skin.

**Changing the skin on the reconnect screen.** When the game closes and the client shows Reconnect, the skin window
opens again with the current pick marked. The running patcher stays armed for that pick, so reconnecting without a
change loads the same skin. Picking another skin or chroma rebuilds the overlay and re-arms the patcher before the
Reconnect click; clearing the pick disarms it. Registration in the client is skipped in this phase, because there
is no champion select to register into. Back in game, the hook is confirmed as on a normal start. With the "Inject
skins" option off, the window stays closed.

**Why the loading screen shows the default name for an unowned skin.** The name on the loading card comes
from the skin id the server received. The client refuses to register a skin you do not own, so the card
shows the champion's default name while the skin's art still loads from the overlay. Showing the skin's name
would require writing to game memory, which Dekan deliberately does not do.

### Modes where champions are picked in the lobby

Swiftplay (queue 480), Quickplay (490) and Brawl have no champion select: the player picks a champion for each
position in the lobby and the match starts straight after the queue pops. Dekan recognises them by the queue id
(480, 490) or the game mode (`SWIFTPLAY`, `BRAWL`) the lobby reports; player slots left over in another queue
never turn a draft or ARAM lobby into one. A lobby's picks are kept through the match even when the client deletes
the lobby as the game starts, and dropped between matches.

```text
Client opens a lobby that picks champions
  └─ dekan-lcu publishes the queue and the champions in the player slots (lobby events, and a read on connect)
     └─ the selection window opens with the lobby, before any queue
        · no champion yet: it says to pick them in the lobby
        · two champions: one tab each; a skin is kept per champion, and saved skins are restored for both
        └─ the trigger arms one patcher with the skins of both champions, already in the lobby
           └─ each skin is registered in its player slot (owned as itself, unowned as the default)
Queue pops, match starts
  └─ the live champion is read from the gameflow session
     └─ the armed patcher covers it, whichever position the match gave; its skin loads
```

### Saved skins: presets, profiles and history

| Source | Set by | Restored when |
| --- | --- | --- |
| Preset | The pin button next to the search box, for the chosen skin | Champion select or lobby, nothing chosen and the default skin in the client |
| History | Every confirmed injection | Same, when the champion has no preset in the active profile |
| Random | The dice, or the control panel option at lock-in | Lock-in (or queueing in a lobby mode) with nothing chosen |

A profile is a named set of presets (for example one for ranked and one for fun). The profile row lists them with
the default first; the plus button creates an empty one and switches to it, and the bin deletes the active one.
Switching profiles replaces a skin that was restored automatically; a skin picked by hand stays. Presets live in
`state\presets.json`, history in `state\historic.json`.

### Game modes

Every mode with a champion select (draft, blind, ranked, ARAM and its variants, Arena, URF, ARURF, One for All,
Ultimate Spellbook, Nexus Blitz, co-op, custom, Practice Tool) uses the flow above. Swarm also has a champion
select, but whether its champions load the regular champion archives could not be checked while the mode is out
of rotation (the game ships a mode's map only while it is live). The local player is
found by their cell, so Arena's duos and One for All's shared pick are handled, and a champion changed after
lock-in (ARAM bench, One for All) drops the old skin and restores the new champion's. Paths that a map archive
holds are changed in whichever map the installed game ships (Summoner's Rift, Howling Abyss, Arena and rotating
maps alike). The `Lobby queue` log line records each queue's id, game mode and map, which is what proves a mode
in a real match.

## 2. Custom mods

```text
The player drops or imports a .fantome, .zip or .modpkg into %LOCALAPPDATA%\Dekan\custom_mods\<category>
  └─ a .modpkg is unpacked at staging into the same layout (META/info.json + WAD/<name>.wad.client, base layer)
  └─ the mod appears in the Mods tab and is selected there
     └─ the selection joins the other mods for the next build
        └─ compatibility check: every data file the mod links to must exist in the game or in the mod
           · a shared skin file (<Champion>_Multi_Skins_<slots>.bin) is renamed by the game whenever a patch adds
             a skin to the group; a link to the old name is replaced by the one file the installed game's skin
             bins link with the same slots plus only slots the mod never mentioned. Nothing else in the bin
             changes. No such file, or more than one → the link stays dangling
           · a texture or mesh path the game no longer has is replaced by the first of: the same path without
             its variant suffix (Name.SKINS_Yone_Skin74.tex → Name.tex), its .dds/.tex or .sco/.scb twin, or
             both, only if the game has it; the mod's own files always win
           · unpacked WAD folders (WAD/<name>.wad.client/...) are checked and repaired like archives
           · a dangling link means the mod was made for an older patch → it is dropped with a warning
             (it would otherwise crash the loading screen)
        └─ property types: a mod made before the game turned text paths into file references (STRING → FILE,
           the XXH64 of the lowercase path) gets those values converted, using the types the installed game
           declares for the same object class and field in the bins the mod replaces, their links and the
           champion's animation graph (an older mod otherwise crashes the game while loading)
        └─ the overlay builder merges what is left, with the same rules as for skins
```

Large mods that touch many game archives (a map overhaul can touch over a hundred) are expensive to build
inside champion select. Dropping entries identical to the game's keeps most of them manageable. Showing the
build cost when the mod is selected is still open work.

## 3. Classic Rift

```text
A classic champion is detected (champion ids offset by 60000, skin ids by 60000000)
  └─ the catalog lists the client's own Classic skins for that champion (Classic-only skins included), keeping
     those the game has a Classic skin file for
  └─ dekan-classic builds the mod from the installed game, under the Classic character the client names
     (Wukong's is jade_wukong, not a name derived from the archive)
     · classic characters are matched by name in the archive's table of contents
     · without a hash table, the champion's data files are scanned to find them
```

The classic ids are kept as they are. They are never forced back to the regular champion.

## 4. Party mode

```text
Friends sit in the same League lobby, each running Dekan
  └─ every Dekan derives the same room id from the lobby's party id and joins the relay quietly
     └─ when another member is in the room (lobby or end of game only),
        Dekan asks "Join the party" / "Don't join"
        └─ after "Join the party", each member announces champion and skin, encrypted on their own machine
           └─ the trigger keeps only announcements whose champion matches the real roster
              reported by the client for that player's slot
              └─ each teammate's skin is generated like your own and merged into the same overlay
                 └─ you see your friends' skins in the match
```

Nothing is announced, read or shown before the player agrees. "Don't join", closing the invite or leaving the room
closes the room for that lobby; a new lobby starts over, and a lobby already accepted shares again when its room reopens.
Only lobbies with two or more members that are not custom games get a room (custom lobbies mix both teams and go
past the relay's five members). The room survives the lobby disappearing during the match and closes between
matches. An open invite closes on its own when queueing starts or the lobby changes. Without a configured relay
the lobby room is skipped with a single warning; an unreachable relay logs one warning per run of failures.

A room code still works: creating or joining a room from the control panel takes priority and closes the lobby
room until it is left. The control panel shows the room state; a room you created reads "Party created" until you
leave it. The selection window footer shows how many are in the room and how many teammate skins (checked against
the real roster) will be applied.

Open work: the selection window does not yet show who is in the room or which skins were applied, and the
mode has not been proven with several players in one real match.

## 5. Startup and patches

```text
Dekan starts → finds the game → checks the injector files (missing, or with a foreign or broken signature: the user
is offered the newest LTK Manager release, otherwise its page opens and Dekan stops) → reads the game build
  · new build? cached overlays and locale data are discarded; the archive index revalidates
    itself by file size and modification time
  · once the archive index is ready, at background priority and only while no champion select or match is
    running, every custom mod in custom_mods is checked against the installed game
    (the same relink and compatibility check as at build time). A mod that becomes fully compatible is
    rewritten in place and its original kept under state\mod_originals; one that cannot is left as it is
    and refused at build time. Verdicts are kept per game executable, so a mod is checked again only after
    a patch or when its file changes
  · build newer than the injector DLL supports? the user is warned at startup
  └─ the client observer connects whenever the client opens; with no local skin library,
     the catalog is filled from the client's own skin list
```

## 6. Shutdown and recovery

- Dekan never suspends or opens the game's threads. When the game starts before the injector is armed, the
  late path builds the overlay and arms the injector within a time budget (60 s by default); the hook may then
  land too late, and the log says so.
- A supervised task that dies is logged (`error` for the essential ones) and a panic is logged by the panic
  hook; Dekan does not restart the task, so the log line is the first thing to look for in a report.
- Uninstalling removes logs, state, overlays and generated mods, and asks before deleting the user's own
  skins. `cargo xtask install-audit` checks the result.

## 7. Updates

```text
Dekan runs from an installed copy (the uninstaller sits next to it)
  └─ 15 s after start, then every 6 h, the latest GitHub release is read; a newer one is offered once per version,
     never during ready check, champion select, a match or a reconnect
     └─ "yes": SHA256SUMS and SHA256SUMS.sig are downloaded and the Ed25519 signature is checked
        └─ the installer's SHA-256 must match the signed list; the file is checked again right before it runs
           └─ when no match is running, Dekan closes, the installer runs silently and reopens Dekan
"no", or a failed check: nothing runs; a failed verification opens the release page
```

A copy not installed by the setup only gets a notice with the download page. `DEKAN_UPDATE_CHECK=off` disables
the check.
