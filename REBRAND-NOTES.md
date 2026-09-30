# Dekan rebrand notes

This package is a full branding fork of the original Bullet 1.1 source tree.

Changed for Dekan:

- application/product name and visible UI text
- executable name (`dekan.exe`)
- installer name (`Dekan-Setup-<version>-x64.exe`)
- all internal Rust crate/package names (`dekan-*`)
- application data paths (`%LOCALAPPDATA%\Dekan`)
- environment variables (`DEKAN_*`)
- mutex, registry autostart name, installer paths and uninstall cleanup
- party code/protocol branding (`DEKAN1` / `dekan-party`)
- relay service package branding
- CI/release artifact names
- root/app icons and PNG branding assets using the supplied Dekan logo
- Windows version-resource product/company metadata

Compatibility note: the existing public relay endpoint is retained as the default URL so party mode does not point at a nonexistent server. You can replace it in `crates/dekan-party/src/config.rs` after deploying your own relay.

The original MIT license and upstream attribution are intentionally preserved. See `LICENSE` and `UPSTREAM-NOTICE.md`.

## Build

The original project targets Windows x64 with Rust/MSVC. Typical commands:

```powershell
cargo xtask check
cargo xtask package
cargo xtask installer
```

The installer script is `installer/dekan.iss` and the final installer is expected under `dist/installer/`.
