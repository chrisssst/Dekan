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
| `archive/wad.rs` | WAD reader: open a whole archive or just its table of contents, read an entry decompressed or as stored |
| `archive/writer.rs` | WAD writer used to build overlays |
| `properties/prop.rs` | BIN/PROP parser and serializer, including the list of linked files; walks every field of an object (`flatten_fields`, `diff_fields`, `field_value`) with bounded depth for the byte-level records; records the installed game's field types (`record_field_shapes`) and converts a mod's text paths into the file references the game declares (`strings_to_files`) |
| `hashing/hash.rs` | Path hashing (xxHash64) and content checksums (XXH3) |
| `archive/fantome.rs` | Reading `.fantome` packages |
| `archive/modpkg.rs` | Reading `.modpkg` packages (league-mod format v1): the table index is streamed, only the `base` layer is mounted, each chunk is bounds-checked and its XXH3 verified, and the stored bytes pass through as WAD chunks |
| `hashing/hash_index.rs` | Hash-to-path lookup table |

## Design notes

- **Untrusted input never panics.** Every offset and length read from a file is checked against the file's
  real size. A truncated or malicious archive returns a typed error and never crashes Dekan.
- **The writer keeps the game's own bytes.** When an entry is copied unchanged, the writer keeps the exact
  compressed bytes the game shipped with and does not decompress and recompress them. The game validates its
  archives. A recompressed copy can be rejected as corrupt even when the content is identical, so keeping the
  original bytes is what lets the overlay load.
- **Duplicates are found by real content.** When the writer merges identical data, it compares the bytes it
  actually wrote. It never trusts the checksum recorded in the game's table of contents.
- **Strict property files.** A PROP version below 2 is refused and bytes after the last object are an error: a
  file that does not end where its own table says is not one this parser understands. Serializing fails
  instead of truncating a value that does not fit its on-disk width.
- **Format changes are legible.** A WAD with a new major version is reported as such, so a patch that changes
  the format reads as that in the log instead of "the skin did not load".
- **Writes are atomic and sequential.** `X.wad.client` is written as `X.wad.client.partial` first; payloads read
  from files are laid out in file and offset order so reads stay sequential. Audio banks (`.bnk`, `.wpk`) are
  stored uncompressed like the game stores them; `r3d2` followed by a model or animation tag is not audio.

## Testing

```powershell
cargo test -p dekan-wad
```

The unit tests use small generated archives. The `xtask` commands `wad-probe`, `wad-types` and
`wad-writer-probe` run the same code against every archive of an installed game.
