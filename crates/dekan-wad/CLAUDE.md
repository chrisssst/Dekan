# dekan-wad

Files, formats and responsibilities: `README.md`. Decision: ADR-021.

- This parser stays our own (no `cdragon-*`) for full control over all five entry types and the bounds checks.
- Every offset, length and count read from a file is bounds-checked and capped before allocating; out of range is a
  typed error, never a panic. The fuzzer found an allocation abort in the PROP header once; new readers get a fuzz
  target and a property test.
- The writer keeps the game's header and unchanged entries' compressed bytes exactly (see
  `crates/dekan-inject/CLAUDE.md`), so a round trip through the writer reproduces every entry
  (`cargo xtask wad-writer-probe`).
