# dekan-platform

Files and responsibilities: `README.md`. Interface decision: ADR-035.

## Interface (Slint)

- One `dekan-ui` thread owns the Slint event loop; logic lives in Rust (`overlay_model/mod.rs`), the `.slint` files
  only lay out and bind.
- The overlay never takes focus on its own. It is created without activation and shown with `SW_SHOWNOACTIVATE`;
  only a click in its search box takes the foreground (thread-input attach), and Enter or Escape gives it back to
  the client, because stealing focus breaks champion select.
- Every control has an accessible label; the headless UI tests find controls by it
  (`overlay_window/tests.rs`), so a control without one is untestable.
- One visual language across every window: navy surfaces with a top highlight (`Surface`), a gold radial `Halo`
  behind the emblem, serif titles, segmented tabs, sentence case, no emoji or font glyphs as icons (vector `Path`).
  Only what the software renderer draws: no transforms, drop shadows or rounded clips. The Slint attribution is the
  README badge, not inside the app.

## Discovery and text

- Game, client and tools paths come from discovery here (`paths`), never a literal drive or folder: users install
  League on any drive, including WeGame layouts.
- Every user-facing string goes through `i18n` (Turkish and English).
