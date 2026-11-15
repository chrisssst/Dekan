# Dekan 2.1 local build
The local preview contains the rebuilt public Windows application and its installer. The installer uses a separate per-user installation path and does not automatically launch the app.
The source comes from the supplied Bullet 1.10 public archive; the Dekan logo, color themes, Turkish texts, Discord application and party relay configuration come from Dekan 2.0 (72411ab).
The original setup and proprietary native engine are not run, changed or repackaged.
## Windows build
Install the Rust toolchain declared in rust-toolchain.toml and the MSVC Windows build tools.
```powershell
cargo build --locked -p dekan-app --bin dekan --release --target x86_64-pc-windows-msvc
```
The local preview uses x86_64-pc-windows-gnu (opt level 1, no LTO, one codegen unit, incremental off), and the NSIS recipe in installer/dekan-preview.nsi. NSIS expects PAYLOAD_DIR to contain the rebuilt dekan.exe, dekan.ico, LICENSE.txt, THIRD-PARTY-NOTICES.md and ONCE_OKU.txt.
## Game-enabled release prerequisites
- A Dekan-owned GitHub OAuth app with Device Flow enabled, with its public Client ID provided at build time via DEKAN_GITHUB_CLIENT_ID. The verification target is chrisssst/Dekan. The login check remains enabled.
- A compatible authorized signed engine build, with its certificate publisher configured via DEKAN_NATIVE_PUBLISHER at build time. Signature verification fails when the publisher is not configured. The separately licensed engine must be obtained through its authorized distribution channel. It is not in this package.
- Validate compatibility of that engine with this app and the current League patch on Windows before distribution. The installer preview is not that validation.
The source package does not publish a release or run release automation.

The installer shows a Dekan-branded setup terms page and ships the complete original MIT notice as LICENSE.txt.
