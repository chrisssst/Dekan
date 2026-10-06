# dekan-classic

Generates skin mods directly from the game you have installed. No pre-built skin package is needed: the skin
is extracted from the game's own archives for the current patch, so it always matches the game version.

## Where it sits in the flow

When you pick a skin, Dekan needs a mod that makes the champion load that skin in place of the default one.
This crate produces that mod. It then goes to `dekan-inject`, which merges it into the overlay.

It covers two cases.

**Store skins.** `StandardChampion` opens the champion's archive (`DATA/FINAL/Champions/<Name>.wad.client`) and
takes the chosen skin's files. It rewrites them to take the default skin's place: in the game's data, the
skin's definition is retargeted from `SkinN` to `Skin0`. Companion characters (pets, summons, alternate forms,
such as Orianna's ball or Zed's shadow) are carried along so they match. They are found by scanning the
champion's property files for `characters/<name>/` references, cached per archive, and kept when the archive
holds a skin file for them. A chroma that has no companion file of its own uses its base skin's. If a
companion cannot be converted, the error is logged and never ignored.

The converted file is the game's own `SkinN` definition, byte for byte, under the default skin's key: it keeps
every file the original links to (the skin's animation graph, effects and shared data live there) and is
marked as a base skin, as the game's default skin always is. The default skin's animation graph is never
replaced; the converted skin keeps pointing at its own.

A companion file that a map archive also holds (Zed's shadow is also in `Map11.wad.client`) is written into the
map archive too by the overlay builder, so both agree. `cargo xtask skin-audit` lists every skin where this
happens.

**Classic Rift.** Some queues use older, "classic" versions of champions. Their ids are offset (champions by
60000, skins by 60000000) to tell them apart. `ClassicChampion` finds these characters inside the game
archives and builds the mod for them. When no hash table is available, it scans the champion's data files to
find the characters.

## What is inside

| File | Purpose |
| --- | --- |
| `generator.rs` | `StandardChampion` and `ClassicChampion`: open a champion from the installed game and build the mod for a skin |
| `builder.rs` | `ClassicIdMapper`: converts between classic and regular champion and skin ids |
| `forms.rs` | Bakes one form of a skin with gears into the slot-0 skin and strips HUD gear indicators |
| `gear_toggle.rs` | Adds the in-game form cycle (`Ctrl+5`) to the animation graph of a skin with gears |
| `clip_alias.rs` | Gives a skin's graph the spell clip the default skin's animations ask for, when the skin only has its own variants |
| `error.rs` | Error type |

## Design notes

- **Built from the local game, per patch.** The mod always matches the installed game, which avoids the most
  common failure of pre-built skin packages: breaking after a patch.
- **Champion names are validated** before they are used in any path, so a malformed name cannot escape the
  output folder.
- **Moving a skin to another slot is a relocation.** The skin object and its resolver get the slot's keys, and
  every `link` or `hash` value that pointed at the old keys, at any depth, follows them (`objectPath`, the resolver
  link and any other reference). A plain number with the same value is data and is never touched.
- **The slot keeps the identity the game gives it.** `skinClassification` and `skinParent` come from the game's
  own bin for that slot (a base skin: classification 1, no parent), so a chroma loaded as slot 0 no longer
  claims to be a chroma of its parent. Nothing in this is a fixed value or a list of skins.
- **Forms cycle in game with `Ctrl+5`.** Some skins come with several forms (gears), such as one sword per class.
  The game switches them only for a skin the server knows the player owns, and the match runs the generated
  skin as the default one, so the gear switch never happens. When every form shows a part no other form shows,
  the skin's own animation graph gets a `Toggle` clip, the one `Ctrl+5` plays: a chain of conditions on which
  of those parts is visible, each leading to a copy of the next form's equip animation (or of the idle when the
  form has none) that shows that form's parts and hides the others. The game's clips are kept byte for byte;
  only new clips are added. A graph that already has `Toggle`, the default skin's own graph, forms that differ
  only in materials, and champions whose default skin has gears too (the game switches those itself during the
  match, as Kayn's transformation) are left as they are. Materials and persistent effects that the skin switches by
  gear (`HasGearDynamicMaterialBoolDriver`) are rewired to "this form's part is visible", in the generated
  skin and in the skin's own bin, so they follow `Ctrl+5` too; this happens only when every such driver names a
  form the skin has (an omitted index is the first form), otherwise the drivers stay as the game wrote them.
  Bins shared by several skins are never rewritten. Effects that a form swaps through its resolver stay those of
  the first form. The game's built-in HUD indicators for gear forms (icons above the champion portrait) are
  stripped from the generated skin data when the toggle is handled by the animation graph.
- **A spell clip the skin renamed is aliased, only on proof.** The match runs the skin under the default id, so
  the game asks the skin's graph for the default clip names (`Spell3` for the third spell). A few skins replace a
  spell clip with variants of their own and have no clip under the default name (Fallen God-King Garen's E: three
  spin speeds, no `Spell3`), which left the champion standing still. The default name gets the variant at normal
  speed, only when every variant is a top-level parallel clip with exactly one clip on the track named after the
  missing clip, there are at least two of them, and every one fires a sound named after that spell slot's spell in
  the champion's own record (`GarenE`). Recall or respawn clips on the same track never qualify.
- **Companion characters are indexed once per champion and patch.** Every builder of the same champion and game
  archive shares one scan (a second caller waits for the first instead of scanning again), and at startup every
  champion is indexed in the background, one at a time, paused while a match runs, with the result cached on disk.
  The first champion select after a patch no longer waits for the scan.
- **Skin scripts the game runs by skin id do not run.** Some skins have a script of their own that the game
  starts only for the owner's skin id (special idle behaviours, music switching, effects reacting to the match).
  The generated skin loads under the default skin's id, so those scripts stay off. The script logic is game
  behaviour, not skin data, and is never edited.
- **A chroma's parent comes from the game.** Its `skinParent` names the skin whose companions it borrows when it
  has none of its own; the client's answer is only a fallback. Companions such as Zoe's orbs or Syndra's spheres
  follow a chroma even when the client is not running.

## Testing

```powershell
cargo test -p dekan-classic
cargo xtask classic-probe Zed Ahri    # builds real mods from the installed game
```
