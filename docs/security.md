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

## Hashes are the trust anchor

Every external binary is checked against the **SHA-256 of an audited copy** before it touches the game. The
check never relies on who signed the certificate (`dekan_inject::dll_validator`). The audited hashes are
constants in `dekan_app::trigger`. The packaging tool reads them from that source file, so the installer and
the app always agree on what is trusted. A byte-patched, unsigned build of the DLL failed in a real match
and is not accepted.

The release pipeline adds another anchor: every installer carries a build provenance attestation that ties it
to the workflow and the commit that produced it.

## Non-Rust dependencies

- `ltk_patcher_host.exe` and `ltk_patcher_dll.dll`, the injection backend. The DLL refuses game builds newer
  than a fixed timestamp (`0x6ac1f970`, 2026-10-04 07:00 UTC). The limit applies to the game build, not to
  the clock. Its bytes are never modified and its signature is never stripped.
- They are not included in the installer. Users take them from an official LTK Manager release or from the
  convenience mirror linked in the README. Either way, only the audited hashes are accepted, so a tampered
  download is refused.
- None of them may be loaded from another product's folder. If one is missing, the user is told the exact
  path Dekan expected.

## Invariants in the code

- Nothing is written to the game folder.
- Extracting a `.fantome` or zip checks every entry path component by component, refuses symlinks, and
  limits total size and entry count to stop zip bombs.
- Dekan never runs elevated. External resources are released by owning guards.
- State files are written atomically, and a file handle is closed before its file is replaced (Windows keeps
  open files locked).
- Party mode never sends account ids or names. The relay sees a room id derived from the invite key and
  encrypted blobs, nothing else.

## Supply chain

- Every GitHub Action is pinned to a commit SHA, and Dependabot proposes updates.
- `cargo deny` blocks crates with known advisories, disallowed licenses or unknown sources. New crates need a
  written justification.
- CodeQL, gitleaks and dependency review run on every pull request, and weekly.
- Changes to workflows, the installer, trusted hashes, the injector code paths or the party cipher never
  merge automatically.

## Antivirus and SmartScreen

The techniques Dekan depends on (`SetWindowsHookEx`, hooking `CreateFileA`, patching in memory) are exactly
what antivirus heuristics flag. Combined with an unsigned installer, many users will see a SmartScreen
warning. The real fix is code signing, which is a prerequisite for wide distribution.

## Reporting a vulnerability

Please use the private security channel configured for your Dekan repository and not a
public issue.
