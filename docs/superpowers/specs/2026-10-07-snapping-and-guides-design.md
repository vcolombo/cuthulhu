<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Snapping and smart guides — design

Date: 2026-10-07

The next step of the editor-first push, after the viewport and handles (#298). Lining a decal up
with the artboard edge, centring text on a shape, or butting two pieces together is most of the
placing work in a cut job. Today it is done by eye or by typing numbers into X/Y/W/H. Snapping
makes it a drag.

## Where it starts

From `main` at `e371ab5`:

- Every canvas gesture reduces to one world-space matrix from `gestureMatrix(kind, box, start, cur,
  mods)` (`interaction/gesture.ts`). The hook re-derives it on every pointer move and every view
  change through `follow()`. Preview and commit use the same matrix.
- The renderer's overlay (`render/Renderer.ts`) draws the selection box, handles and the marquee
  band, in screen space.
- `expand(ids)` gives every shape a selection actually moves, Groups included.
- There is no grid and no snapping of any kind. Shift's 15° rotation steps are the only quantising.

## Decisions

- **Snapping adjusts the pointer, not the matrix.** Before `gestureMatrix` runs, the point the
  gesture would move is pulled onto the nearest target. Then the matrix is computed as it is today.
  Preview, commit, Shift and Alt all keep working unchanged, and "what you release is what is
  saved" still holds.
- **What snaps:**
  - **Move:** the moving box's left, centre and right lines snap on x, and its top, middle and
    bottom lines snap on y. Each axis is independent. With Shift's axis lock, only the free axis
    snaps.
  - **Scale, on an axis-aligned box:** only the edge or edges the handle moves snap. That covers
    every multi-selection, whose union box is always axis-aligned, and every unrotated single
    shape. With Alt, the moving edge still snaps and its mirror follows.
  - **Not snapped:** rotation, which has Shift's 15° steps; the marquee; and scaling a rotated
    single shape. (`// ponytail:` its edges are not axis-aligned, so "the edge at x = 30" has no
    meaning for it. Ceiling: rotated shapes scale freely. Upgrade: snap along the box's own axes
    to targets projected into its frame.)
- **What it snaps to:**
  - the artboard's left, centre and right lines, and its top, middle and bottom lines;
  - every other shape's world bounds, by the same six lines.

  Shapes being moved are excluded, so a selection never snaps to itself. A rotated target
  contributes its axis-aligned bounds. (`// ponytail:` its outline is not consulted.)
- **Targets are gathered once per gesture**, when the press lands. Nothing but the selection moves
  during a gesture, so the target lists stay valid for its whole length. They are sorted per axis,
  and each pointer move is a binary search.
- **Tolerance is 6 CSS px**, converted to mm by the view scale at use, like the handle sizes. A
  zoom mid-drag changes the reach, and `follow()` already re-derives the gesture on a view change.
- **The nearest target wins, per axis.** When two of the box's lines are equally close to targets,
  the centre line wins, then the lower coordinate, so the choice does not flicker between frames.
- **Hold ⌘ (Ctrl on Windows and Linux) to drag without snapping.** It is read per move, so it can
  be pressed mid-drag. Shift and Alt are taken; ⌘ is free during a drag. Snapping is on by
  default. There is no toggle in the UI yet. (`// ponytail:` a View menu toggle once there is a
  menu to put it in.)
- **Guides are drawn for every axis that snapped.** Each is a 1 px line in a new `--guide` colour
  token, a third colour distinct from cyan (selection) and red (cut). A guide runs along the
  snapped coordinate across the span of the moving box and every target sharing that line. An
  artboard snap spans the artboard. Guides show only while a gesture is snapped, and they go away
  on release. A guide is drawn only if the landed box really touches its line: the matrix can
  discard part of a snap (Shift sizing from the other axis, the minimum-size clamp), and a Shift
  move holds its locked axis at the start so a snap cannot flip the lock.

## Structure

```
ui/src/interaction/
  snap.ts          NEW  targets(scene, excluded, artboard) -> sorted per-axis lines;
                        snapMove / snapScale(box, targets, tol, mods) -> adjusted point + guides
  gesture.ts            unchanged; snapping happens before it, on the pointer
  useCanvasInteraction.ts
                        gathers targets on press; follow() snaps the pointer (unless ⌘ is held)
                        before gestureMatrix; passes guides to the overlay
ui/src/render/
  Renderer.ts           Overlay gains `guides: Guide[]` (world-space segments)
  Canvas2DRenderer.ts   draws guides in screen space, 1 px, --guide
ui/src/tokens.css       the --guide token, beside --accent and --cut (one theme today)
```

No IPC change, so `ipc-inventory.json` and `Cargo.lock` do not move. `dist/` is rebuilt and
committed.

## Testing

- **vitest** (`snap.test.ts`):
  - targets exclude the moving shapes and include the artboard lines;
  - move snaps each axis to the nearest line within tolerance and leaves it alone outside;
  - the tie-break is stable (centre first, then the lower coordinate);
  - Shift's axis lock snaps only the free axis;
  - scale snaps only the moving edge, Alt included;
  - a rotated single box does not scale-snap;
  - guide segments span the box and the matching targets;
  - tolerance scales with zoom.
- **Playwright** (fake backend):
  - dragging the red rect so its right edge stops 0.4 mm short of the Group's rect commits a move
    that butts them exactly (e = 20, not 19.6), and so does a press-and-drag on an unselected rect;
  - the same drag with Ctrl held commits 19.6;
  - an `e`-handle scale stopping 0.4 mm short snaps its edge (×3, not ×2.96), and with Alt mirrors
    the other edge (×5);
  - a move snaps to the artboard's centre line;
  - a selected Group does not snap to its own shape.
- **Manual** (`MANUAL-CHECKLIST.md`): the guides read clearly against the artboard and the
  workspace, and snapping feels sticky but not grabby at fit and at 400%.

## Out of scope

- Snapping to a grid. There is no grid yet.
- Equal-spacing and distance guides ("these three are 5 mm apart").
- Snapping during rotation, and scale-snapping rotated shapes (see the ponytail above).
- Keyboard nudge with arrow keys.
- Snapping preferences: per-target toggles, and setting the tolerance.
- Align and distribute commands. That is the next spec, and it will reuse `snap.ts`'s target lines.
