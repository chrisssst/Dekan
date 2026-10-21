# Dekan 1.4 rebrand notes

Dekan maintains its own product identity and release configuration.

Customized for Dekan:

- application/product name, executable and installer names
- Rust crate/package names (`dekan-*`)
- `%LOCALAPPDATA%\Dekan`, registry/autostart names and `DEKAN_*` variables
- party/protocol branding (`DEKAN1` / `dekan-party`) using the Dekan Cloudflare relay and migrating the former default
- Windows version metadata, installer metadata and GitHub workflow branding
- supplied Dekan logo for app, installer and the Slint UI
- interface languages reduced to Turkish and English
- Turkish is the default/fallback interface language; English remains available for English locales
- Discord community links point to `https://discord.gg/dekan`
- existing Dekan release/promote/star-notification workflow customizations are retained

Dekan uses the MIT license in `LICENSE`; third-party dependency terms remain in `THIRD-PARTY-NOTICES.md`.

The package intentionally omits `assets/urgot-select.png` and `assets/urgot-ingame.png`; when overlaying it onto the existing Dekan repository, keep the current Dekan README screenshots already in `main`.
