# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/) and
versions follow [Semantic Versioning](https://semver.org/). Detailed notes for each build are on the
[Releases](https://github.com/Isllanrx/Bullet/releases) page.

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
- Tray application with English, Portuguese and Spanish, start with Windows, single instance.
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

[1.1]: https://github.com/Isllanrx/Bullet/releases/tag/v1.1
[1.0.0]: https://github.com/Isllanrx/Bullet/releases/tag/v1.0.0
