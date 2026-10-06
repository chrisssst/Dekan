# Security policy

## Reporting a vulnerability

Please report security issues **privately** through
[GitHub Security Advisories](https://github.com/chrisssst/Dekan/security/advisories/new). Do not open a
public issue or post details on Discord.

Useful details to include:

- the Dekan version (Explorer → `dekan.exe` → Properties → Details),
- what an attacker could do, and under which conditions,
- steps to reproduce, and the log from `%LOCALAPPDATA%\Dekan\logs` if relevant.

Maintainer: Dekan.

You will get an acknowledgement within a few days. Fixes ship as a new pre-release first and are credited in
the release notes unless you prefer otherwise.

## Scope

In scope:

- `dekan.exe`, the installer and the uninstaller,
- the party mode protocol, its encryption and the relay (`relay-worker/`, `crates/dekan-relay`),
- the build and release pipeline in `.github/`.

Out of scope:

- the LTK patcher binaries (report those to [League Toolkit](https://github.com/LeagueToolkit/ltk-manager)),
- the League of Legends client or game,
- detection or penalties by Riot's anti-cheat. Using Dekan is at your own risk, as the README's disclaimer
  explains.

## Supported versions

Only the latest release and the latest pre-release receive security fixes.

## Verifying a download

Every installer published by the release workflow comes with a `SHA256SUMS` file and a build provenance
attestation:

```powershell
Get-FileHash .\Dekan-Setup-<version>-x64.exe -Algorithm SHA256
gh attestation verify .\Dekan-Setup-<version>-x64.exe --repo Isllanrx/Dekan
```
