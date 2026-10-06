Dekan is a customized fork of **Bullet** by Isllan Toso. The original Bullet source is MIT licensed; the original copyright notice is preserved in `LICENSE`. See `UPSTREAM-NOTICE.md` for provenance.

# Third-party notices

Dekan's own source code is licensed under the [MIT License](LICENSE). This file lists the third-party
components Dekan uses or depends on and the terms they come with.

## LTK patcher (injector)

`ltk_patcher_host.exe` and `ltk_patcher_dll.dll` are made by the League Toolkit organization and ship with
[LTK Manager](https://github.com/LeagueToolkit/ltk-manager). They are **not** part of this repository, are
**not** covered by Dekan's MIT license, and are **not** included in Dekan's installer. The README links an
official way to get them (LTK Manager) and a convenience download mirror, which is not an official League
Toolkit release.

They are governed by the
[LTK Patcher License](https://github.com/LeagueToolkit/ltk-manager/blob/main/LTK-PATCHER-LICENSE.md), published by
League Toolkit. In short, it allows using, studying and building software that uses the binaries. Anyone who
redistributes them outside an unmodified official LTK Manager release must remove League Toolkit's code
signature and may only sign them with their own certificate. The official way to get them is to copy both
files from an LTK Manager release. Dekan only runs copies whose SHA-256 matches an audited build, whatever
their source.

For questions about these binaries or their license, use the contact published by League Toolkit in that
license. Dekan is not affiliated with or endorsed by League Toolkit.

## Rust crates

`dekan.exe` statically links open-source Rust crates from [crates.io](https://crates.io). Every crate in the
dependency graph uses a permissive license. The accepted set is enforced in CI by `cargo deny` (see
`deny.toml`):

| License | Crates (approx.) |
| --- | --- |
| MIT and/or Apache-2.0 (dual or single) | ~175 |
| Unicode-3.0 | 19 |
| BSD-3-Clause, BSD-2-Clause, 0BSD | 5 |
| ISC | 5 |
| Zlib | 4 |
| CDLA-Permissive-2.0 (Mozilla CA bundle via `webpki-roots`) | 2 |
| BSL-1.0 | 2 |
| Unlicense, CC0-1.0 or MIT-0 (as an option) | 4 |

For the exact list with versions and license texts, run:

```powershell
cargo deny list          # every crate and its license
cargo tree --edges normal --target x86_64-pc-windows-msvc
```

The Apache-2.0 and BSD licenses require their copyright notices to travel with binaries. Those notices are
included in each crate's source, which is available from crates.io at the versions pinned in `Cargo.lock`.

## Other components

| Component | Use | License |
| --- | --- | --- |
| [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) | Renders Dekan's selection window; installed separately, not redistributed | Microsoft Software License Terms |
| WebView2 SDK loader (via `webview2-com`) | Linked into `dekan.exe` to locate the runtime | BSD-3-Clause (Microsoft) |
| [Inno Setup](https://jrsoftware.org/isinfo.php) | Builds the Windows installer | Inno Setup License (permissive) |
| `relay-worker` dev tooling (`wrangler`, `typescript`, `@cloudflare/workers-types`) | Build and deploy only; not shipped to users | MIT / Apache-2.0 |

## Reference projects

Dekan was built by studying the projects listed in the README's acknowledgements. No source code from them
is included. Where a reference is GPL-licensed (for example LTK Manager), Dekan reimplements the behavior it
needs independently.

## Trademarks

League of Legends and Riot Games are trademarks or registered trademarks of Riot Games, Inc. Dekan is not
affiliated with, endorsed or sponsored by Riot Games or League Toolkit.
