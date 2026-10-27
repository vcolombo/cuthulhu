<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Align and distribute — design

Date: 2026-10-07

The third step of the editor-first push, after the viewport and handles (#298) and snapping
(#300). Snapping lines one piece up with another as you drag. Align and distribute do it for a
whole selection in one click: line up the left edges of six name decals, centre a logo on the
artboard, or space a row of letters evenly for weeding.

## Where it starts

From `main` at `5dfde3b`:

- `commit_transform(ids, m)` applies **one** world-space matrix to every id, as one `Delta` and so
  one undo entry (`apps/desktop/src/state.rs`, `document::commands::transform_nodes`).
- Every canvas transform goes through the hook's one send path (`useCanvasInteraction.ts`), with
  its in-flight rule, pending preview, revision-based retirement and per-field queue.
- `expand(ids)` gives the shapes a selected id moves; `snap.ts`'s `boxBounds` and
  `transformBounds` give world bounds.

## Decisions

- **Six align commands and two distribute commands.**
  - Align: left, horizontal centre, right, top, vertical middle, bottom.
  - Distribute: equal horizontal spacing and equal vertical spacing. Spacing means equal gaps
    between neighbours, not equal centre-to-centre steps, because gaps are what a weeder works
    between.
- **The unit that moves is a selected id.** A selected Group moves as one piece, by the bounds of
  all the shapes under it (`expand`), just as a drag moves it. A selected shape inside a selected
  Group is not moved twice: the backend already skips a node whose ancestor is selected.
- **What it aligns to:**
  - With two or more units selected, align to the selection's own bounds: "left" moves every unit
    to the leftmost edge.
  - With one unit selected, align to the artboard: "centre" centres it on the mat.

  Both behaviours are what the operator expects from the buttons alone, with no mode to set.
  (`// ponytail:` no key unit (Illustrator's "key object") or "align to artboard" toggle for
  multi-selections. Ceiling: you cannot align five pieces to a sixth that stays put. Upgrade: a key
  unit, chosen by clicking a selected unit again, as in Illustrator.)
- **Distribute needs three or more units.** Unless one unit is a frame (below), the two outermost
  stay put: the unit that starts first and the unit that reaches furthest. The others move between them so the gaps between neighbours
  are equal, ordered by their left (or top) edge, with document order breaking ties. (Revised in
  gate 1: taking the far end from the unit that starts last threw small pieces past each other when
  a wide one started first.) If the units together are wider than the span, the gaps come out
  negative, so they overlap evenly; that is still the arithmetic answer, and no special case hides
  it, as long as every piece, the far end included, is longer than the overlap, so the landed
  starts and ends keep strictly increasing. When a piece is no longer than the overlap it would
  be passed by the next one or left level with it, the neighbouring gaps stop being equal, and each
  further click would pick a different first piece, so distribute is disabled with the reason
  instead (Copilot on #301).
- **A unit that spans all the others is a frame.** Selecting a whole design selects its weed
  border or backing plate too. That unit stays put and the others are spaced inside it, with the
  margins at its edges equal to the gaps between them. Sure Cuts A Lot 6 offers this as a separate
  "Distribute to Selection Below" mode (manual §3.19); Silhouette Studio has no equivalent. Here
  the selection says which is meant, so there is no mode to set. When more than one unit spans
  the rest (stacked copies of one shape), none of them is the frame and distribute is disabled; it
  is also disabled when the pieces are longer than the frame, since spacing them evenly would push
  the outer ones through it. Edges are compared to within 1e-6 mm, so a piece flush with its border
  counts as inside it despite float noise.
  (Decided 2026-10-07, after gate 1 had first disabled distribute whenever a unit spanned.)
  Inside a frame, distribute is disabled too when a piece spans the other pieces in turn, as a weed
  border on a backing plate does with both selected: spaced as one more piece, the border left its
  letters outside it. Spacing them inside the innermost frame is the likely upgrade, but it is a
  choice of layout, so for now the reason says to deselect one. And the pieces-longer-than-the-gap
  rule for two ends holds inside a frame as well, where the margin is the gap: a line of no width
  in a frame its pieces fill would otherwise land level with a neighbour and move on every click.
  (Review of #301, 2026-10-08.)
- **Moves are axis-locked translations.** Horizontal commands change x only and vertical commands
  change y only. A rotated unit aligns by its axis-aligned bounds, the same convention snapping
  uses. (`// ponytail:` not by its outline. Ceiling: a rotated piece's outline sits off the line by
  however far its box overhangs it. Upgrade: line up each unit's outline extreme along the axis.)
  A path's bounds also take in its curves' control points (`pathBounds`), so a curve whose control
  points overshoot it lines up by the overshoot; that predates this step and is tracked apart from it.
- **One click, one undo entry.** Each unit needs its own translation, which `commit_transform`
  cannot express, so the backend gains **`commit_transforms(moves: [{ ids, m }])`**. It runs
  `transform_nodes` for each entry against the document as it stands, concatenates the deltas,
  and commits them as one `Delta`. It refuses the whole batch if any entry is refused, so a
  half-aligned selection is never saved. Units that would not move are left out, and an empty
  batch commits nothing.
- **Through the hook's send path, like every transform.** It extends the X/Y/W/H mechanism: the
  hook's `transformEach(key, make)` builds the moves from the effective scene when they are sent.
  It queues behind a commit on the wire, keyed by kind, axis and selection like the fields, so a
  newer align on the same axis replaces a queued one, while "Align top" does not replace "Align
  left" and a distribute does not replace an align.
  (Revised in gate 1, from a single `align` key.) The pending preview applies each move to its
  own shapes. The commit callback carries a list, and a single-move
  gesture becomes a list of one.
- **UI:** a row of six align buttons and a row of two distribute buttons in the properties panel,
  under X/Y/W/H. They are disabled when they cannot act: align with nothing selected, distribute
  with fewer than three units. Units are counted from the shapes on the canvas, so an empty Group
  is not one, and distribute is also disabled on an axis where several units span the rest, with a
  tooltip giving the reason. Each has an `aria-label` naming it, with icons from the panel's
  existing line style. There are no keyboard shortcuts in this step.

## Structure

```
crates/document/src/commands.rs   transform_each(doc, moves) -> Delta  (one Delta, all-or-nothing)
apps/desktop/src/state.rs          commit_transforms(moves)
apps/desktop/src/ipc.rs            #[tauri::command] commit_transforms(moves)
apps/desktop/src/main.rs           registered in generate_handler!
apps/desktop/ipc-inventory.json    "commit_transforms": ["moves"]
ui/src/ipc.ts                      commitTransforms
ui/src/interaction/align.ts   NEW  alignMoves / distributeMoves(units, mode, artboard) -> moves
ui/src/interaction/useCanvasInteraction.ts
                                   send() takes a list of moves; transformEach(key, make)
ui/src/App.tsx                     commit callback sends the list; align handlers
ui/src/panels/PropertiesPanel.tsx  the align and distribute rows
ui/e2e/smoke.spec.ts               the fake gains commit_transforms, mirroring transform_each
```

`commit_transform` stays as it is: the e2e fake and its tests still invoke it, though the frontend no
longer does (the CLI never called a Tauri command). A single-move list goes through the new command.

`ipc-inventory.json` is regenerated by `UPDATE_IPC_INVENTORY=1 cargo test -p desktop --test
ipc_inventory`. That test builds the desktop crate, which needs WebKit, and this development
container cannot build it. Here the entry is written by hand, and CI's `rust` job is the check
that it matches the registry.

## Testing

- **Rust** (`document`):
  - `transform_each` with two entries yields one `Delta`, and undoing it restores both;
  - a refused entry (an unknown id) refuses the batch and changes nothing;
  - entries see the effects of earlier entries in the same batch;
  - a node under a selected ancestor in another entry is not moved twice.
- **vitest** (`align.test.ts`):
  - each of the six aligns, for two or more units, against the selection bounds;
  - a single unit aligns to the artboard;
  - units that would not move are left out;
  - distribute keeps the outermost units and equalises the gaps (including the overlapping case),
    breaks ties by document order, and refuses fewer than three units;
  - a Group unit uses all its shapes' bounds.
- **Playwright** (fake backend):
  - "Align left" on the two seeded rects plus the Group commits one batch that moves only the
    Group, as one undo;
  - "Align horizontal centres" on a single selected rect centres it on the artboard;
  - distribute on three units equalises the gaps;
  - the buttons are disabled when they cannot act;
  - an align queued behind a held drag lands from where the drag left the shapes.
- **Manual** (`MANUAL-CHECKLIST.md`): six names aligned left cut in a straight column on the Cameo
  5, and distributed letters weed with even gaps.

## Out of scope

- A key unit, and an align-to-artboard toggle for multi-selections (the ponytail above).
- Distribute by centres, and match-size commands ("make same width").
- Keyboard shortcuts for align and distribute.
- Aligning by outline rather than bounds.
- A toolbar placement for the buttons. They live in the properties panel until the toolbar is
  redesigned.
