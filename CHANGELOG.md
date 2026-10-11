# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/) and
versions follow [Semantic Versioning](https://semver.org/). Detailed notes for each build are on the
[Releases](https://github.com/chrisssst/Dekan/releases) page.

## [Unreleased]

### Added

- Custom mods broken by a patch are repaired at startup: when a patch adds a skin and renames the files skins
  share (`<Champion>_Multi_Skins_*.bin`, or the older `DATA/<Champion>_Skins_*.bin`), each reference is pointed at
  its single successor in the installed game and the mod file is updated in place; the original is kept under
  `state\mod_originals`. Texture and mesh paths the patch renamed (a variant suffix such as
  `.SKINS_Yone_Skin74` dropped, or a `.dds`/`.tex` twin) are pointed at the file the game has now. The scan runs
  at background priority and waits while a champion select or match is running. A mod that cannot be fully
  matched is left untouched and left out, as before.
- Unpacked WAD folders inside a mod are now checked before injection, like packed ones.
- A tray notice and a **Custom mods** line in the control panel tell how many mods were adjusted or left out;
  **Restore original mods** in the panel puts the kept originals back (only for files that are still the
  repaired ones) and leaves those files unrewritten.
- `.modpkg` mods are checked at startup too, and the startup check reports asset formats (models, skeletons,
  animations, textures, sound banks) the installed game no longer uses.

### Fixed

- A skin form picked with `Ctrl+5` (such as one of Viego's Revenant swords) no longer goes back to the first
  form when the champion walks or casts, and its ability, basic attack and idle effects, part materials and
  gear-picked animations follow the form too (Viego Revenant, Diana, Katarina, Sett, Morgana and the other skins
  whose forms the game switches by gear).
- Skins whose forms each have their own model (Elementalist Lux's ten elements) now cycle with `Ctrl+5` too: the
  forms are merged into one model generated from the installed game, each with its own texture. A player who owns
  the skin in the same match keeps the original model.
- Switching form with `Ctrl+5` plays that form's own transformation (animation, effects, sound and voice line) when
  the skin has one, and ability sounds and voice lines follow the form on skins whose sounds change with the form.
- A skin generated once is reused while the game and Dekan are unchanged, so champion select no longer regenerates
  heavy skins (such as Spirit Blossom Morgana) and the patcher is ready before the game starts. Picking the same skin
  again while it is being generated waits for that generation instead of failing.

- A saved skin (pinned or last used) is no longer restored over a custom skin selected for the champion, and
  picking a custom skin drops a saved skin restored a moment before.
- `cargo xtask mod-audit`: runs that repair on copies of mods and checks each one entry by entry against the
  original and the installed game.

## [1.2] — 2026-09-30

### Added

- Control panel: click the tray icon for options, party, folders and a diagnostics list (injector, game,
  client, injector DLL deadline).
- A random skin is rolled when your champion locks in with none chosen (can be turned off in the panel).
- New version notice: a Windows notification and a download link in the panel when a new release is out.
  Nothing is downloaded or installed automatically; `DEKAN_UPDATE_CHECK=0` turns the check off.
- Diagnostics for skin reports: every generated skin and overlay archive is recorded field by field, the match is
  followed through the game's local live data, and when a match ends the logs, records and the screenshots taken
  in that match (F12) are zipped into the logs folder. `Ctrl+Shift+B` marks a problem during a match without
  leaving the game. Nothing reads or writes the game's memory, and other players' names are never written.
- Skins with several forms (a sword per class, an outfit per stage) cycle them in game with `Ctrl+5`, when each
  form has a part of its own; the form's materials and lasting effects follow it.

### Fixed

- Every generated skin is consistent with the slot it takes: the skin names itself and its resources by the
  slot's keys, and a chroma loaded as the default skin no longer claims to be a chroma of another skin.
- Chromas keep their companions (Zoe's orbs, Syndra's spheres, Nasus's ultimate, Ivern's totem, Fizz's
  bait, Anivia's wall) even when the League client does not answer: the parent skin is read from the game.
- After creating a party, the control panel says "Party created" with the room count instead of the same
  line a joined room shows, so the disabled Create and Join buttons no longer look like a frozen window.
- Zed's shadow and the companions of 895 other skins (Shaco, Syndra, Taliyah, Yorick and more) now take the
  skin's look: files a map also holds are changed in the map too.
- Skins keep their own animations; the default skin's animation graph is no longer replaced.
- Chromas are loaded as the game loads its default skin.
- Orianna's ball and every other companion character follow the chosen skin.
- Every skin tier the client lists can be chosen (such as Immortalized Legend), and Rift Classic offers every
  Classic skin, including Wukong's.
- Champion names, forms and companions are read from your installed client instead of fixed tables.
- Opening Dekan a second time shows the control panel instead of a blocking message.
- Fallen God-King and God-King Garen spin with their own animation on E instead of standing still.
- The game is never hooked while it is already loading (that could crash it); the skin loads on the reconnect
  instead.
- The first champion select after a patch no longer waits for the champion's files to be scanned: every champion
  is indexed in the background once per patch, and simultaneous builds share one scan.
- Chroma previews no longer hold the selection: they are fetched together as soon as the champion's list opens,
  and a click on a skin is taken at once even while previews are still arriving.
- Clicking Create several times while a party window was open no longer opens several rooms and windows.
- The first champion select after a patch no longer freezes the selection window while a map archive is
  prepared.

### Security

- The released `dekan.exe` links the C runtime statically again (it needed the Visual C++ runtime installed)
  and no longer carries paths of the machine that built it; its PE checksum is set.
- Dekan no longer suspends the game or asks for the debug privilege.
- Dekan refuses to start without the audited injector and says where to put it.
- Hardened the game file parsers against malformed data found by fuzzing.
- The installer's version details are complete, and releases are code-signed once signing is enabled.

## [1.1] — 2026-09-28

### Added

- Chroma preview when hovering a chroma in the selection window.
- The selection window can be resized from its corner; wide windows show skins in columns.
- Optional automatic match accept, toggled from the tray (off by default).

### Fixed

- Legendary and mythic skins keep their own animations and effects (for example God-King Garen's E).
- The search box in the selection window receives typing again, and search ignores accents.
- The About window no longer cuts its subtitle or shows an empty item, and can be resized.

### Security

- Releases ship the provenance bundle (`.sigstore.json`) next to the installer for offline verification.
- Party mode refuses to start instead of crashing if the system random generator fails.
- Cryptography and WebSocket dependencies updated (`sha2` 0.11, `chacha20poly1305` 0.11,
  `tokio-tungstenite` 0.30).

## [1.0.0] — 2026-09-25

First public release.

### Added

- Skin selection in Dekan's own window, attached to the League client. No client plugins.
- Store skins generated from the installed game on each patch, including companion characters.
- Native, byte-faithful overlay builder: unchanged archive entries keep the game's exact bytes, which the
  current patch requires.
- Injector armed during champion select, before the game process exists.
- Custom `.fantome` mods in ten categories, with a compatibility check that drops mods broken by a patch.
- Classic Rift models generated from the installed game.
- Party mode: teammates see each other's skins through an encrypted relay that cannot read the content.
- Tray application with English, Turkish, start with Windows, single instance.
- Installer and uninstaller with an install audit, file details and a manifest that runs Dekan without
  administrator rights.

### Security

- Runs unelevated; never writes to the game folder.
- Injector binaries verified by SHA-256 against audited builds before use.
- CI with CodeQL, secret scanning, dependency review, `cargo deny`, workflow auditing, OpenSSF Scorecard,
  pinned actions and build provenance attestations for every installer.

### Known limitations

- The LTK injector is not bundled in the installer. Users copy it from LTK Manager 1.21.0 or 1.22.0, which ship
  the audited build, or from the direct download linked in the README.

- Proven in live matches for Draft and Ranked; other modes are still being validated.
- The injector DLL refuses game builds made after 2026-10-04 07:00 UTC; a refreshed DLL is needed then.

[1.2]: https://github.com/chrisssst/Dekan/releases/tag/v1.2
[1.1]: https://github.com/chrisssst/Dekan/releases/tag/v1.1
[1.0.0]: https://github.com/chrisssst/Dekan/releases/tag/v1.0.0
