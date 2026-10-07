# dekan-inject

Files and responsibilities: `README.md`. Trust model: `docs/security.md` and ADR-036.

## Overlay

- The overlay is built byte-faithful: unchanged entries keep the game's exact compressed bytes and every cloned WAD
  keeps the game's header (signature and checksum). Patch 16.19 rejected a map WAD rewritten with another header or
  recompressed as corrupt (`Map11.wad.client`).
- A path a map WAD also holds changes in every WAD that holds it. Changing one side only is the "Inconsistent"
  crash; leaving it out left Zed's shadow on its default look.
- A WAD whose mod only replaces entries is the game file copied byte for byte plus the new entries. The copy is
  reused until the game file changes and prepared when the champion locks: rewriting a 2.5 GB map takes 24 s on an
  SSD and 53 s on a 5,400 rpm drive, and the game starts before an unarmed patcher. Nothing outside the cache
  (`overlay_cache.rs`, the `.base` stamps) may delete that copy; the uninstaller is the one exception.

## Injector

- Third-party binaries come only from Dekan's own `tools` folder and only with a valid Authenticode signature from
  `LTK_PUBLISHER` (`trust.rs`). The signature survives LTK's per-patch DLLs; any changed byte breaks it and is
  refused. No version or hash is hardcoded. Changing the publisher needs a new ADR and the README's "Step 2".
- Never patch the DLL bytes or strip its signature, and never bundle the binaries: the LTK Patcher License forbids
  redistributing them outside an official LTK Manager release. Their absence is told to the user.
- LTK Manager itself installs to `C:\Program Files\LTK Manager`; Dekan never loads from there, it copies into its
  own `tools`.
