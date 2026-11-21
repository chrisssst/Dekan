# ADR-032: Use the supplied native bridge bundle for Dekan 2.2

## Context

The maintainer reports that the Dekan 2.1 bridge is obsolete and supplied a newer launcher/DLL pair. The newer public source archive has no working companion implementation; its release workflow imports it from a private repository. The maintainer authorized internal compatibility identifiers to remain unchanged while the visible product and public tool names remain Dekan.

## Decision

Retire the old bridge and launcher workspace crates. Package the supplied x64 binaries from `vendor/native` instead of compiling or selecting the old executables from `target`. Pin their bytes, check them before packaging and launch, and retain optional native publisher verification.

Keep `dekan_launcher.exe` and `dekan_bridge.dll` as the public tool names. The launcher's fixed DLL basename requires an identical internal `bullet_bridge.dll` copy next to it; create it during packaging and install it with the hidden attribute. Include it in checksums, runtime verification and install/uninstall auditing.

Discover the supplied bridge through its actual profile directory and `BulletBridge` hello identity. Do not reuse the old `DekanBridge` service during upgrade. Keep token authentication and Mod Manager version checks. Require users to close the old Mod Manager process before upgrading.

## Consequences

The UI, release signing, star gate, themes, locales, party configuration and update identity remain Dekan. Native internals are a binary integration, not newly reconstructed source. They cannot be given new internal version metadata or renamed protocol identifiers without source. Vendor hashes prevent silent replacement by stale build outputs.

No new dependency or security bypass is introduced. Loopback protocol and packaging tests establish the adapter behavior; Windows loading and a real match are separate required checks before a production release.
