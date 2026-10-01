# dekan-classic

Generates skin mods directly from the game you have installed. No pre-built skin package is needed: the skin
is extracted from the game's own archives for the current patch, so it always matches the game version.

## Where it sits in the flow

When you pick a skin, Dekan needs a mod that makes the champion load that skin in place of the default one.
This crate produces that mod. It then goes to `dekan-inject`, which merges it into the overlay.

It covers two cases.

**Store skins.** `StandardChampion` opens the champion's archive (`DATA/FINAL/Champions/<Name>.wad.client`) and
takes the chosen skin's files. It rewrites them to take the default skin's place: in the game's data, the
skin's definition is retargeted from `SkinN` to `Skin0`. Companion characters (pets, summons, alternate forms)
are carried along so they match. If a companion cannot be converted, the error is logged and never ignored.

**Classic Rift.** Some queues use older, "classic" versions of champions. Their ids are offset (champions by
60000, skins by 60000000) to tell them apart. `ClassicChampion` finds these characters inside the game
archives and builds the mod for them. When no hash table is available, it scans the champion's data files to
find the characters.

## What is inside

| File | Purpose |
| --- | --- |
| `generator.rs` | `StandardChampion` and `ClassicChampion`: open a champion from the installed game and build the mod for a skin |
| `builder.rs` | `ClassicIdMapper`: converts between classic and regular champion and skin ids |
| `error.rs` | Error type |

## Design notes

- **Built from the local game, per patch.** The mod always matches the installed game, which avoids the most
  common failure of pre-built skin packages: breaking after a patch.
- **Champion names are validated** before they are used in any path, so a malformed name cannot escape the
  output folder.

## Testing

```powershell
cargo test -p dekan-classic
cargo xtask classic-probe Zed Ahri    # builds real mods from the installed game
```
