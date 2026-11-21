# Dekan native bridge bundle

These Windows x64 payloads were supplied by the maintainer for the 2.2 integration. They are not rebuilt from the retired Dekan 2.1 bridge/launcher source. Their bytes are retained exactly; the public filenames are `dekan_launcher.exe` and `dekan_bridge.dll`.

`cargo xtask package` verifies the pinned SHA-256 digests and copies these files into `dist/tools`. It also creates an identical `bullet_bridge.dll` compatibility copy: the supplied launcher requires that DLL basename. Inno Setup marks this internal copy hidden. Do not remove the alias, patch the binary strings, substitute old build output or sign the payloads in place without deliberately updating the release manifest and pins.

The supplied DLL uses `BulletBridge` for its loopback handshake and publishes its per-session authentication token under the corresponding local profile directory. Dekan uses that internal identity but retains Dekan branding in the application, installer, icons and public tool names. Authentication is retained; no fixed token is embedded.

`manifest.json` records hashes and provenance, not a digital signature or runtime success claim. Optional configured native publisher verification is retained. These binaries have not been executed in a Windows match in this environment.
