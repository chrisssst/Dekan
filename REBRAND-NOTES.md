# Dekan 1.3 rebrand notes

This package is based on the upstream Bullet 1.3 source tree.

Customized for Dekan:

- application/product name, executable and installer names
- Rust crate/package names (`dekan-*`)
- `%LOCALAPPDATA%\Dekan`, registry/autostart names and `DEKAN_*` variables
- party/protocol branding (`DEKAN1` / `dekan-party`) while retaining the upstream public relay endpoint for compatibility
- Windows version metadata, installer metadata and GitHub workflow branding
- supplied Dekan logo for app, installer and the new Slint UI
- interface languages reduced to Turkish and English
- Turkish is the default/fallback interface language; English remains available for English locales
- Discord community links point to `https://discord.gg/kutsal`

The original MIT license and upstream attribution are preserved in `LICENSE` and `UPSTREAM-NOTICE.md`.

- The package intentionally omits `assets/urgot-select.png` and `assets/urgot-ingame.png`; when overlaying it onto the existing Dekan repository, keep the current Dekan README screenshots already in `main`.
