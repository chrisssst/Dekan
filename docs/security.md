# Security and trust

## What Dekan injects

Dekan **does inject third-party code into the game process**; it is not "injection-free". The injector DLL:

- is loaded into `League of Legends.exe` through `SetWindowsHookEx`,
- hooks `CreateFileA` so that reads of the rebuilt archives go to the overlay,
- bypasses the game's integrity check of its archives.

It does not change game objects, gameplay memory or rendering. Tools that did that are the ones that were
detected and banned. Dekan belongs to the same family as cslol-manager and LTK Manager: cosmetic changes
through replaced files.

## Honest risk

- **It is not guaranteed safe.** Studying the anti-cheat's logs showed no sign against Dekan, but that is
  an absence of evidence in the logs that were read, not a guarantee. Using it breaks Riot's Terms of
  Service and can lead to a ban at any time.
- It is **less risky** than tools that write to game memory, not "safe".
- Showing an unowned skin's name on the loading screen would require writing to game memory, so Dekan does
  not do it.

## The publisher's signature is the trust anchor

Both injector files are checked before they touch the game: their **Authenticode signature** must be valid
(`WinVerifyTrust`) and the signer must be League Toolkit's publisher, `Natoken LLC`
(`dekan_inject::trust::LTK_PUBLISHER`). Changing a single byte breaks the signature, so a patched or tampered
build is refused; the byte-patched "2040" DLL that failed in a real match is unsigned and refused the same way.
Revocation is not fetched online (`WTD_REVOKE_NONE`), so the check works offline and never stalls champion
select.

The signature replaced a list of audited SHA-256 hashes (ADR-036). League Toolkit now publishes a DLL per game
patch; with fixed hashes every patch needed a Dekan release before users could inject again, while the
publisher's signature accepts each new official build the day it ships. The game-build limit is not a
constant either: it is read from the DLL's own build comparison (`dekan_inject::trust::dll_build_limit`) and
treated as unknown when the pattern is absent or ambiguous.

The release pipeline adds another anchor: every installer carries a build provenance attestation that ties it
to the workflow and the commit that produced it.

## Non-Rust dependencies

- `ltk_patcher_host.exe` and `ltk_patcher_dll.dll`, the injection backend. The DLL refuses game builds newer
  than the timestamp compiled into it (read from the DLL; `0x6ad46e70`, 2026-10-18 07:00 UTC, for the LTK Manager
  1.27.0 build). The limit applies to the game build, not to the clock. Its bytes are never modified and its signature is never stripped.
- They are not included in the installer or in `dekan.exe`. Dekan can download them at the user's request
  from the official LTK Manager repository at the newest release tag whose files carry the publisher's
  signature (`injector_install.rs`); users can also take them from an LTK Manager release or the convenience
  mirror linked in the README. Either way, only files signed by the publisher are accepted, so a tampered
  download is refused.
- None of them may be loaded from another product's folder. If one is missing, the user is told the exact
  path Dekan expected.

## Invariants in the code

- Nothing is written to the game folder (`Game/`).
- Extracting a `.fantome` or zip checks every entry path component by component, refuses symlinks, and
  limits total size and entry count to stop zip bombs.
- Dekan runs unelevated. The one exception is the injector copy: when Windows denies writing to the
  `tools` folder, Dekan starts itself elevated with `--install-injector <staging> <tools>`. That instance
  writes the staged files next to their targets as `.partial`, verifies the publisher's signature on exactly
  those files, accepts only Dekan's own `tools` folders as the target, renames them into place and exits; a
  refused file leaves the old ones untouched. It never starts
  the app. External resources are released by owning guards.
- The only file Dekan changes in the League install is one key of `Config/LeagueClientSettings.yaml`
  (`install.crash_reporting.enabled: false`), and only while **Light match loading** is on.
- State files are written atomically, and a file handle is closed before its file is replaced (Windows keeps
  open files locked).
- Updates are announced, never applied: Dekan reads the tag of the latest published release from the GitHub
  API, compares it with its own version and shows a notice. It downloads no file and runs nothing; the notice
  opens the release page in the browser, and only `https://` pages can be opened.
- Party mode never sends account ids or names. The relay sees a room id derived from the invite key and
  encrypted blobs, nothing else.

## Supply chain

- Every GitHub Action is pinned to a commit SHA, and Dependabot proposes updates.
- `cargo deny` blocks crates with known advisories, disallowed licenses or unknown sources. New crates need a
  written justification.
- CodeQL, gitleaks and dependency review run on every pull request, and weekly.
- Changes to workflows, the installer, the injector trust check, the injector code paths or the party cipher never
  merge automatically.

## Antivirus and SmartScreen

The techniques Dekan depends on (`SetWindowsHookEx`, hooking `CreateFileA`, patching in memory) are exactly
what antivirus heuristics flag. Combined with an unsigned installer, many users will see a SmartScreen
warning. The real fix is code signing, which is a prerequisite for wide distribution.

## Reporting a vulnerability

Please use [GitHub Security Advisories](https://github.com/chrisssst/Dekan/security/advisories/new) and not a
public issue.
