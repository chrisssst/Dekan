# dekan-core

The shared vocabulary of Dekan. Every other crate speaks in the types defined here, and this crate depends on
no other Dekan crate. It contains no `unsafe` code and no I/O beyond what its types need.

## Where it sits in the flow

Everything Dekan knows at a given moment lives in a single `AppState` value: the game phase, your team, the
mods you selected, the injection status, who is in your party. Other components do not share mutable
structures. The state is published through a Tokio `watch` channel, so a reader always sees the latest
complete snapshot and never a half-updated one.

The state only changes through named transitions defined in `state.rs` (for example "phase changed" or
"injection finished"). Nothing else can write to it. When something looks wrong in a log, there is a short,
known list of places that could have caused it.

## What is inside

| File | Purpose |
| --- | --- |
| `state.rs` | `AppState`, its sender/receiver types and the named transitions that change it |
| `phase.rs` | `GamePhase`: turns the client's gameflow phases (lobby, champ select, game start, reconnect, end of game…) into one enum |
| `supervisor.rs` | `Supervisor`: every background task is started here with a cancellation token, so shutdown is orderly and a crashed task is noticed |
| `champions.rs` | Every champion: numeric id, the name used by its game archive, and its companion characters (Annie's Tibbers, Ivern's Daisy, and so on) |
| `selection.rs`, `forms.rs` | What the user chose, including champions with alternate forms |
| `historic.rs` | The last skin used on each champion, so it can be selected again automatically |
| `mods.rs` | Custom mod categories (skins, maps, fonts, announcers, UI, voiceover, loading screens, VFX, SFX, others) and which of them allow only one active mod |
| `library.rs` | The skin library on disk and how an entry is found for a champion and skin |
| `overlay.rs` | Messages exchanged between Dekan and its selection window |
| `party.rs` | Checks the skins announced by party members against the real team roster |
| `env.rs` | The only list of `DEKAN_*` environment variables Dekan reads |
| `error.rs` | Error types shared across crates |

## Custom mods

- Mods are read only from `%LOCALAPPDATA%\Dekan\custom_mods`; Dekan never reads another product's mod folder.
- A mod folder is `META/info.json` plus a non-empty `WAD/` or `RAW/`, matched case-insensitively like Windows
  does; `.fantome` and `.zip` archives are extracted into the staging directory. Anything else is counted and
  reported once, so a folder dropped in with the wrong layout does not vanish without a trace. Hidden
  `.<name>-import-*` temporaries that mod managers leave after a failed import are not mods.
- `description.txt` is read up to a tooltip-sized limit.
- What the selection window sends is validated per slot: every id must be listed in the slot it was sent for
  (a map id sent as the font is refused, an unknown id is never guessed at). The accepted part is applied, the
  refused part is logged, and the window receives the effective selection back.
- A chroma preview request carries only the chroma id; the image path is resolved from the catalog Rust built,
  never taken from the page.

## Party verification

`party.rs` decides whether a teammate's announced skin can be trusted. An announcement is only accepted when
the player's champion matches the one the League client reports for that player's slot. A party member cannot
make you load a skin for a champion they are not playing, and a spoofed message has no effect.

## Testing

```powershell
cargo test -p dekan-core
```

Everything here is pure logic, so the tests run anywhere and need neither the game nor the client.
