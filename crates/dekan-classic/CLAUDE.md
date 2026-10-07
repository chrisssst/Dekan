# dekan-classic

Files and responsibilities: `README.md`. Decisions: ADR-030 to ADR-033.

## Retargeted skin bins

- A retargeted skin bin is the source bin relocated, nothing more. `retarget_skin_bin` keeps only the skin object
  and its Resources, re-keyed to slot 0; every `link`/`hash` value that pointed at the old keys is rewritten to the
  new ones (`remap_references`).
- `skinClassification` and `skinParent` take the values of the game's own bin for the target slot
  (`slot_identity`, never a constant). A chroma's parent comes from its `skinParent`; the client is only a
  fallback (ADR-031).
- `skin0.bin` links `SkinN.bin` and every link `SkinN.bin` had, because the animation graph, VFX and shared bins the
  object references live there. Dropping them froze God-King Garen's sword.
- Never write `animations/skin0.bin`: the skin keeps pointing at `Animations/SkinN`, and a slot-0 graph only
  replaces the base graph other players and companions use.
- Aliases, skin forms and companions come from the client and the game; there are no fixed tables. Classic Rift
  ids are offset (60000 / 60000000) and the base is never forced.

Changes here close with `cargo xtask skin-audit` (0 findings across every skin and chroma) and, for anything a
match would show, a real match.
