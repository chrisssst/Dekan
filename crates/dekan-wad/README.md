# dekan-wad

Reads and writes the file formats League of Legends uses for its game data. This crate depends on no other
Dekan crate and is independent of the game being installed.

## Where it sits in the flow

The game stores almost all of its assets in `.wad.client` archives. Every step of building a skin overlay goes
through this crate:

- the skin generator reads champion archives to extract a skin,
- the overlay builder reads the game's archives and writes the modified copies,
- mod checks parse the game's data files to see what a mod refers to.

## Formats

**WAD (version 3).** An archive with a header, a table of contents and the file data. Files are identified by
the xxHash64 of their lowercase path instead of by name. Each entry is stored in one of five ways: raw,
gzip, a redirect to another path, zstd, or zstd split into chunks. The chunked zstd form covers more than half
of the game's entries, and all five are supported.

**BIN / PROP.** The game's property files, which describe characters, skins, particles and so on. They also
list other `.bin` files they depend on. Dekan uses that list to find out whether a mod still fits the current
patch.

**.fantome.** The community mod package, a zip that holds WAD content and metadata.

**Hash index.** Maps path hashes back to readable paths, which makes diagnostics and some lookups possible.

## What is inside

| File | Purpose |
| --- | --- |
| `wad.rs` | WAD reader: open a whole archive or just its table of contents, read an entry decompressed or as stored |
| `writer.rs` | WAD writer used to build overlays |
| `prop.rs` | BIN/PROP parser and serializer, including the list of linked files |
| `hash.rs` | Path hashing (xxHash64) and content checksums (XXH3) |
| `fantome.rs` | Reading `.fantome` packages |
| `hash_index.rs` | Hash-to-path lookup table |

## Design notes

- **Untrusted input never panics.** Every offset and length read from a file is checked against the file's
  real size. A truncated or malicious archive returns a typed error and never crashes Dekan.
- **The writer keeps the game's own bytes.** When an entry is copied unchanged, the writer keeps the exact
  compressed bytes the game shipped with and does not decompress and recompress them. The game validates its
  archives. A recompressed copy can be rejected as corrupt even when the content is identical, so keeping the
  original bytes is what lets the overlay load.
- **Duplicates are found by real content.** When the writer merges identical data, it compares the bytes it
  actually wrote. It never trusts the checksum recorded in the game's table of contents.

## Testing

```powershell
cargo test -p dekan-wad
```

The unit tests use small generated archives. The `xtask` commands `wad-probe`, `wad-types` and
`wad-writer-probe` run the same code against every archive of an installed game.
