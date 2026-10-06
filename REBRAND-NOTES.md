# Dekan 1.2.1 rebrand notes

This package is based on the upstream Bullet 1.2.1 source tree.

Customized for Dekan:

- application/product name, executable and installer names
- Rust crate/package names (`dekan-*`)
- `%LOCALAPPDATA%\Dekan`, registry/autostart names and `DEKAN_*` variables
- party/protocol branding (`DEKAN1` / `dekan-party`) while retaining the upstream public relay endpoint for compatibility
- Windows version metadata, installer metadata and GitHub workflow branding
- supplied Dekan logo converted to PNG/banner and multi-resolution Windows ICO assets
- interface languages reduced to Turkish and English
- Turkish is the default/fallback interface language; English remains available when an English locale is detected

The original MIT license and upstream attribution are preserved in `LICENSE` and `UPSTREAM-NOTICE.md`.

## 1.2.1 release-ready adjustments

- User-facing language choices are limited to Turkish and English.
- Turkish is the default/fallback UI language.
- Portuguese release-notification text was replaced with Turkish.
- CI executable metadata validation now matches the Dekan-branded binary.
