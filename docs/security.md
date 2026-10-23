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

## The official release is the trust anchor

Both injector files are checked before they touch the game (`dekan_inject::trust::verify_injector_file`):

- signed by League Toolkit's publisher, `Natoken LLC` (`dekan_inject::trust::LTK_PUBLISHER`), with a valid
  Authenticode signature (`WinVerifyTrust`): accepted;
- carrying no signature at all (`TRUST_E_NOSIGNATURE`): accepted, because League Toolkit has shipped official
  releases with an unsigned DLL (LTK Manager 1.29.1 and 1.29.2);
- signed by anyone else, with a broken or tampered signature, or not a binary Windows can read: refused. Changing a
  byte of a signed file breaks its signature, so a patched signed build is still refused.

Dekan downloads the files only from the tag of a published (not draft, not pre-release) release of
`LeagueToolkit/ltk-manager`, over HTTPS from the GitHub API. Without a signature, that origin and the ACL of the
`tools` folder (administrator-only under Program Files) are what protect the files. The tag is resolved to its
commit once, and every file is read at that commit. A copy outside Program Files may fall back to `tools` folders
the user can write; a program already running as that user can replace them, which gains it nothing it could not
already do as that user, but it is the residual risk of an unsigned file. Revocation is not fetched
online (`WTD_REVOKE_NONE`), so the check works offline and never stalls champion select.

League Toolkit also runs a separate delivery service for its own manager, which registers only builds attested by
League Toolkit's release workflow. Dekan does not use it and does not present itself as LTK Manager.

Reading the official release replaced a list of audited SHA-256 hashes. League Toolkit now publishes a DLL per game
patch; with fixed hashes every patch needed a Dekan release before users could inject again, while the
official release accepts each new build the day it ships; Dekan checks for one every 30 minutes and installs it by
itself outside ready check, champion select and the match. The game-build limit is not a
constant either: it is read from the DLL's own build comparison (`dekan_inject::trust::dll_build_limit`) and
treated as unknown when the pattern is absent or ambiguous.

The release pipeline adds another anchor: every installer carries a build provenance attestation that ties it
to the workflow and the commit that produced it.

## Non-Rust dependencies

- `ltk_patcher_host.exe` and `ltk_patcher_dll.dll`, the injection backend. The DLL refuses game builds newer
  than the timestamp compiled into it (read from the DLL; `0x6ad46e70`, 2026-10-18 07:00 UTC, for the LTK Manager
  1.27.0 build). The limit applies to the game build, not to the clock. Its bytes are never modified and its signature is never stripped.
- They are not included in the installer or in `dekan.exe`. Dekan can download them at the user's request
  from the official LTK Manager repository at the newest usable release tag
  (`updates/injector_install.rs`). Valid publisher signatures and official unsigned releases are accepted;
  foreign or broken signatures and DLLs restricted to LTK Manager are refused. The elevated installer resolves
  the tag to its commit and downloads both files itself.
- None of them may be loaded from another product's folder. If one is missing, the user is told the exact
  path Dekan expected.

## Invariants in the code

- Nothing is written to the game folder (`Game/`).
- Extracting a `.fantome` or zip checks every entry path component by component, refuses symlink entries and any
  entry that would be written through a symlink or junction already in the destination, and
  limits total size and entry count to stop zip bombs.
- Dekan runs unelevated. The one exception is the injector copy: when Windows denies writing to the
  `tools` folder, Dekan starts itself elevated with `--install-injector <version> <tools>`. That instance trusts
  nothing the user can write: it accepts only a release version made of digits and dots, resolves the release
  tag to its commit and downloads both files itself at that commit, so the files it checks are the files it
  installs even if the tag moves. The target must be the `tools` folder beside its own executable (never a
  path from an environment variable) and must not be a link or junction. It writes the files next to their
  targets as `.partial`, verifies the publisher's signature on exactly those files, renames them into place
  (restoring the previous files if a rename fails) and exits; a refused file leaves the old ones untouched.
  It never starts
  the app. External resources are released by owning guards.
- The only file Dekan changes in the League install is one key of `Config/LeagueClientSettings.yaml`
  (`install.crash_reporting.enabled: false`), and only while **Light match loading** is on.
- State files are written atomically, and a file handle is closed before its file is replaced (Windows keeps
  open files locked).
- Dekan rewrites a custom mod in `custom_mods` only when the references it can match to the installed game
  make the whole mod resolve; the rewrite is extracted with the same checked routine, written to a temporary
  file, flushed and renamed over the original, and the original is copied once to `state\mod_originals`
  first. Only the link list of the affected property files changes. Nothing outside Dekan's own
  `custom_mods` is ever written.
- Updates run only after the user says yes. Dekan reads the latest published release from the GitHub API and,
  outside champion select and matches, asks once per version. On yes it downloads `SHA256SUMS` and
  `SHA256SUMS.sig`, checks the Ed25519 signature against the public key built into Dekan (the private key is
  a release secret, never in the repository), requires the list to name `Dekan-Setup-<that version>-x64.exe`,
  accepts only assets under the project's `releases/download/` URL and compares the installer's SHA-256 after
  the download and again right before running it, holding the file open without write or delete sharing until
  the installer exits. Any mismatch, or a release without a signature, runs nothing and opens the release page. The
  installer is started unelevated and asks for elevation itself, so it reopens Dekan as the original user;
  Dekan itself never elevates for an update. Copies not installed by the setup only get the notice.
- Party mode never sends account ids or names. The relay sees a room id derived from the invite key and
  encrypted blobs, nothing else. A lobby room's key is a SHA-256 of the lobby's party id with a fixed label: only
  players in that lobby know the id, the relay sees just the derived room id, and nothing is announced until the
  player accepts the invite. The party id is never logged. It is not a strong secret: a player who was invited to
  that party or left it also knows it, and could join the room, see that members use Dekan and read what they
  announce after accepting, or fill the room. The roster check still keeps such a player from changing anyone's
  skin.

## Supply chain

- Every GitHub Action is pinned to a commit SHA, and Dependabot proposes updates.
- `cargo deny` blocks crates with known advisories, disallowed licenses or unknown sources. New crates need a
  written justification.
- CodeQL, gitleaks and dependency review run on every pull request, and weekly.
- Changes to workflows, the installer, the injector trust check, the injector code paths or the party cipher never
  merge automatically.

## Antivirus and SmartScreen

The techniques Dekan depends on (`SetWindowsHookEx`, hooking `CreateFileA`, patching in memory) are exactly
what antivirus heuristics flag. The release pipeline signs `dekan.exe` and the installer with Authenticode when
its signing credentials are configured and fails the release if a signature is not valid; a release built without
them is unsigned, and many users will then see a SmartScreen warning.

## Reporting a vulnerability

Please use [GitHub Security Advisories](https://github.com/chrisssst/Dekan/security/advisories/new) and not a
public issue.
