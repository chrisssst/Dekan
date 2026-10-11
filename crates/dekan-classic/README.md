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
| `generation/generator/mod.rs` | Shared names and paths (aliases, skin and animation bins, slot identity) and the re-exports of the segment folders below |
| `generation/generator/reuse.rs` | One generation per mod folder at a time, and reuse of a generated folder whose manifest matches the champion archive, the Dekan build and the options |
| `generation/generator/slots/retarget.rs` | Relocates a skin's bin and animation graph to slot 0 (`retarget_skin_bin`, `retarget_animation_bin`, form cycles) |
| `generation/generator/rift/characters.rs` | Finds companion and jade characters in a champion's bins, in on-disk order, with the per-WAD cache |
| `generation/generator/rift/classic.rs` | `ClassicChampion`: builds a Classic Rift mod |
| `generation/generator/store/standard.rs` | `StandardChampion`: builds a store skin's mod into slot 0 |
| `generation/generator/store/prewarm.rs` | Lists the champion archives and prewarms the companion scan ahead of champion select |
| `generation/generator/store/aliases.rs` | Resolves a champion id to its game alias from the installed game data |
| `generation/generator/store/missing_clips.rs` | Gives a skin's own graph the clips the default skin's animations ask for |
| `generation/generator/cycle/form_cycle.rs`, `generation/generator/cycle/form_steps.rs`, `generation/generator/forms/form_bake.rs`, `generation/generator/cycle/form_audio.rs` | The skin's forms: the in-game `Ctrl+5` cycle, or one form baked into slot 0 |
| `generation/builder.rs` | Classic id offsets: tells a classic champion id apart and converts classic champion and skin ids back to the regular ones |
| `animation/skin_forms/forms.rs` | Bakes one form of a skin with gears into the slot-0 skin and strips HUD gear indicators |
| `animation/gear/gear_toggle/mod.rs` | Reads a skin's gears (parts to show and hide, equip animation) and rewires gear drivers to the form markers |
| `animation/graph/form_graph/mod.rs` | Builds the form cycle into the animation graph: a copy of every clip per form, picked by marker, and the `Ctrl+5` swap that holds the form on a track of its own |
| `animation/parts/form_mesh/mod.rs`, `generation/generator/forms/form_models.rs` | Forms with their own mesh and skeleton: each gear becomes a gear of parts of one merged model, its whole-model texture becomes per-part overrides, and the skin points at the merged files under new paths |
| `animation/transition/form_transition/mod.rs` | Finds each form's own transformation clip (a contextual situation of the skin whose sounds name the form) so `Ctrl+5` plays it |
| `audio/sound_bank/mod.rs`, `audio/form_sound/mod.rs` | Wwise sound banks read and written with their original objects untouched, and one event per form that sets the skin's `gear_N` sound switches, fired when the form is entered |
| `mesh/formats/skeleton/mod.rs`, `mesh/formats/skinned_mesh/mod.rs`, `mesh/merge/mesh_merge/mod.rs` | Skeletons (`.skl`) and skinned meshes (`.skn` v4) read and written byte for byte, and the merge of several forms into one: shared bones by name, a skinning twin wherever a form binds a bone differently |
| `animation/parts/form_marker/mod.rs` | Adds one invisible marker part per extra form to the skin's mesh and hides the markers at start |
| `animation/gear/form_gear/mod.rs` | Reads what each gear changes besides parts: idle effects per form, materials per part (a part drawn with another material in a form gets a copy for that form) and what a cycle cannot carry |
| `animation/vfx/form_vfx/mod.rs` | Gives every effect that changes by form one system with each form's emitters drawn only under that form's stencil reference |
| `animation/vfx/vfx_markers.rs` | Stencil markers: one invisible ground effect per form, held by the form's persistent condition, that writes the form's stencil reference |
| `animation/clips/form_clips/mod.rs` | Clip helpers for the form graph: strips form parts from the game's events and plays, in each form's copy, the variant the game picks by equipped gear |
| `animation/graph/form_state/mod.rs` | Holds each form through a persistent effect condition of the skin (state clip playing or marker visible) and gives the drivers gears are rewired to |
| `animation/skin_forms/form_trace/mod.rs` | Debug trace of a skin's forms, the game clips that change part visibility and the clips added |
| `animation/clips/clip_alias/mod.rs` | Gives a skin's graph the spell clip the default skin's animations ask for, when the skin only has its own variants |
| `binary.rs` | Bounds-checked little-endian reads shared by the mesh and sound bank parsers |
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
  the skin's own animation graph gets a `Toggle` clip, the one `Ctrl+5` plays: a chain of conditions on the form
  markers (below), each leading to the next form's swap. A graph that already has `Toggle`, the default skin's own graph, forms that differ
  only in materials, and champions whose default skin has gears too (the game switches those itself during the
  match, as Kayn's transformation) are left as they are. Materials and persistent effects that the skin switches by
  gear (`HasGearDynamicMaterialBoolDriver`) are rewired to "this form's part is visible", in the generated
  skin and in the skin's own bin, so they follow `Ctrl+5` too; this happens only when every such driver names a
  form the skin has (an omitted index is the first form), otherwise the drivers stay as the game wrote them.
  Bins shared by several skins are never rewritten. Effects that a form swaps through its resolver stay those of
  the first form. The game's built-in HUD indicators for gear forms (icons above the champion portrait) are
  stripped from the generated skin data when the toggle is handled by the animation graph.
- **The chosen form survives walking and spells.** A part shown by a clip's event belongs to that clip, so the
  form is not kept in the swords themselves but in markers: one invisible part per extra form (three vertices at
  the same point) appended to the skin's mesh and hidden at start; no marker visible is the first form. Only the
  `Ctrl+5` swap shows or hides a marker. Every clip of the game, composite ones included, gets one copy per form
  whose references stay inside that form, and each atomic copy shows its form's parts and hides the others' (never
  a marker; the game's own visibility events lose only the form parts). The clip's own key becomes a chain of
  conditions on the markers, so whatever the game plays by name enters the visible form's copy and a spell never
  switches form halfway. The swap plays, in parallel, a looping clip on a track of its own (`DekanForm`, mask with
  every weight at zero, sized from the game's masks) that keeps the form and its marker, and a copy of the form's
  equip animation (or idle). Gear drivers in the skin's bins read the markers. Loop flag, track, parallel and driver
  fields are the ones the game's own data uses. The graph is checked (every clip, track and mask reference
  resolves, unique keys, a cap on clip count) and a mesh that cannot take the markers (not v4, over 31 parts or
  65,535 vertices) leaves the skin without the cycle. Because the game's own persistent effect conditions
  recompute part visibility whenever a buff or animation changes, each form also gets a persistent effect
  condition in the skin bin, active while the form's state clip plays or its marker is visible, that shows the
  form's parts and marker and hides the others'; it is the mechanism the game itself uses to hold parts under
  a state, and the gear drivers are rewired to the same condition so effects follow the form.
- **Everything a gear changes follows the form.** A gear is what the server switches for a skin owner: its
  parts, its effects resolver, its idle effects, its material per part and its equip animation. Under the
  default skin's id the server never switches it, so each piece is rebuilt from the installed game. Effects: for
  every effect name whose system differs by form, one system holds every form's emitters, each drawn only where
  the stencil holds that form's reference (emitters identical in every form stay once, without stencil); an
  invisible ground effect per form, held by the form's persistent condition, writes that reference, and the first
  form's also starts as an idle effect a second after spawn, because one made on the loading screen does not
  write the stencil. Idle effects move into a persistent condition per form. A part that a form draws with another
  material gets a copy in the mesh (same vertices, its indices repeated) with that material, shown only in that
  form. Clips the game picks by equipped gear play, in each form's copy, that form's variant. What a cycle cannot
  carry is logged and listed by `cargo xtask skin-audit`: a form with another mesh or skeleton turns the cycle off
  for that skin, and a whole-model texture, material or scale per form, and states the skin's own server script
  sets through buffs (such as Viego's Ascension), stay as the first form shows them.
- **A spell clip the skin renamed is aliased, only on proof.** The match runs the skin under the default id, so
  the game asks the skin's graph for the default clip names (`Spell3` for the third spell). A few skins replace a
  spell clip with variants of their own and have no clip under the default name (Fallen God-King Garen's E: three
  spin speeds, no `Spell3`), which left the champion standing still. The default name gets the variant at normal
  speed, only when every variant is a top-level parallel clip with exactly one clip on the track named after the
  missing clip, there are at least two of them, and every one fires a sound named after that spell slot's spell in
  the champion's own record (`GarenE`). Recall or respawn clips on the same track never qualify.
- **Companion characters are indexed once per champion and patch.** Every builder of the same champion and game
  archive shares one scan (a second caller waits for the first instead of scanning again), and at startup every
  champion is indexed one at a time on a thread in Windows background mode (low CPU, disk and memory priority),
  paused from the ready check until the match ends, with the result cached on disk. The caller owns that thread.
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
