<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Viewport and selection handles — design

Date: 2026-10-06

Finishes the part of SP3 (`2026-07-22-editor-shell-design.md`) that shipped as stubs: the V1
tool set promised zoom/pan, scale handles, rotate and marquee, and the canvas has none of them.
It is the first step of the editor-first push: Cuthulhu's case against Silhouette Studio is that
it is fast and uncluttered, and an editor that cannot zoom cannot make that case.

## Where the canvas is today

Read from `apps/desktop/ui/src` on 2026-10-06:

- `<canvas width={800} height={600}>` — a fixed backing store, not sized to its grid cell, and
  not scaled for `devicePixelRatio`, so it is blurry on every Retina display.
- Document millimetres are drawn as CSS pixels 1:1. A Cameo 5 artboard (≈305 mm) is ≈305 px wide
  and cannot be enlarged.
- `hitTest` tests axis-aligned world bounds; `canvasPos` returns canvas pixels and treats them as
  millimetres. Both are only correct because the view is the identity.
- `applyOptimistic` and `dragMatrix` handle translation only.
- `Canvas2DRenderer.draw` builds a `new Path2D(d)` for every path on every frame — re-parsing
  every imported path per mouse-move.
- The ponytail on `lineWidth` already names this work: strokes scale with the node because there
  is no zoom to divide by.

What does **not** need to change: `commit_transform(ids, m)` takes any affine, and
`document::commands::transform_nodes` composes it in world space under transformed ancestors.
`cutplan` flattens outlines through the full world transform. Rotation and non-uniform scale
already reach the blade correctly; this spec is UI work plus one Rust test that pins that claim.

## Decisions

- **The viewport is UI state, not document state.** It never crosses IPC, never enters undo, and
  is not saved in the project. A project opened on a laptop and on a 5K display should not fight
  over a zoom level. (`// ponytail:` if users ask for per-project views, it goes in `manifest.json`
  next to the document, not inside it.)
- **One matrix, `view: Affine6`, maps world mm → CSS px**, uniform scale plus translation. The
  renderer sets `ctx.setTransform(dpr·view)` once per frame and composes each node's `world` on
  top; every pointer event goes through `inverse(view)` exactly once, at the top of the handler.
  Nothing downstream of that line sees a screen coordinate.
- **Gestures follow Figma/Illustrator on a trackpad**, since that is what a Mac user's hands
  already know:
  - two-finger scroll / mouse wheel → pan
  - pinch, or ⌘/Ctrl + wheel → zoom about the cursor (WebKit reports pinch as `ctrlKey` wheel)
  - Space-drag and middle-button drag → pan
  - ⌘0 fit artboard, ⌘1 actual size, ⌘= / ⌘- step zoom
- **"100%" means CSS reference size**: 96 CSS px per CSS inch (`25.4 / 96` mm per px), which
  is close to 1 mm on screen per 1 mm on the mat but exact only at the display's reference
  density and OS scaling (Copilot on #298; per-monitor calibration is a follow-up). Fit is the default on launch, project open and machine switch.
  Zoom is clamped to [fit / 4, 6400%].
- **Strokes and handles are screen-constant.** Line width, handle size and hit tolerance are
  stated in CSS px and divided by the view scale, resolving the existing ponytail.
- **Hit-testing moves to the node's own frame.** A pointer is mapped through `inverse(world)` and
  tested against the shape's local box with a tolerance converted to local units. That is exact
  for rects, correct for rotated shapes (an axis-aligned box of a 45° shape is twice its area),
  and costs one matrix inverse per candidate. (`// ponytail:` box, not outline — clicking the hole
  of a letter "O" selects it. Upgrade path: `ctx.isPointInStroke` on the cached `Path2D` with a
  widened `lineWidth`, once someone reports it.)
- **Handles act on the selection box, in world space.**
  - One node selected: the box is the node's *oriented* local box, so a rotated rect keeps
    handles that sit on its edges.
  - Several selected: the axis-aligned union of their world bounds.
  - 8 scale handles (corners + edge midpoints); a rotate zone just outside each corner (cursor
    changes, no extra glyph); dragging inside the box moves.
  - Modifiers: Shift constrains (proportional scale, 15° rotate steps, axis-locked move);
    Alt/Option scales about the centre instead of the opposite handle.
  - Scale that would cross zero clamps at 0.1 mm in that axis. No flip-by-drag in V1 — a negative
    determinant is a mirrored cut, and that should be a deliberate command, not an overshoot.
- **One gesture, one `commit_transform`, one undo entry** — unchanged from SP3. Every gesture
  produces a single world-space `Matrix`; the optimistic preview and the commit use the same one,
  so what you release is what is saved.
- **Marquee on empty canvas.** Drag from empty space draws a rubber band; release selects nodes
  whose world bounds intersect it, Shift adds. A click without movement still clears the
  selection. It is in scope because handles make multi-select common, and Shift-clicking 40
  imported letters is the kind of chore this product exists to remove.
- **`Path2D` is cached per node**, keyed on the path string, so a frame re-uses parsed geometry.
  The renderer drops entries whose node left the scene.

## Structure

Pure modules carry the maths and get the unit tests; `App.tsx` only wires events to them. That is
the dialog split (`viewmodel.ts` + thin `.tsx`) applied to the canvas.

```
ui/src/
  interaction/
    viewport.ts     NEW  view matrix: fit, zoomAt, panBy, screenToWorld, clamp
    gesture.ts      NEW  (handle, startPt, curPt, box, modifiers) -> Matrix
    selectionBox.ts NEW  oriented/union box and its handle positions, in world space
    marquee.ts      NEW  rect-vs-bounds selection
    transform.ts         applyOptimistic generalised from translate to full affine
  render/
    affine.ts       NEW  compose/apply/invert/scaleAbout/rotateAbout — App.tsx's private copies, shared
    hittest.ts           local-frame test with a world-space tolerance
    Canvas2DRenderer.ts  setView(), DPR backing store, Path2D cache, handles + marquee overlay
  App.tsx                ResizeObserver on the canvas cell; pointer events -> the modules above
  panels/StatusBar.tsx   zoom % and cursor x,y in mm (both already in SP3's layout)
```

`App.tsx` is 556 lines today. The pointer wiring moves into a `useCanvasInteraction` hook rather
than growing it further.

No IPC command is added or changed, so `ipc-inventory.json` does not move. `dist/` does, and is
rebuilt and committed with the change.

## Testing

- **vitest** (pure maths):
  - `viewport`: `screenToWorld(worldToScreen(p)) == p`; `zoomAt` keeps the point under the cursor
    fixed; clamps hold; fit centres the artboard with a margin.
  - `gesture`: each handle with and without Shift/Alt yields the expected matrix; opposite handle
    stays fixed; rotate about the box centre; 15° snapping; the 0.1 mm clamp; a zero-length drag
    is the identity (and the caller commits nothing, as today).
  - `selectionBox`: a 30°-rotated rect's handles lie on its edges; the union box of two nodes.
  - `hittest`: inside/outside a rotated rect near its corner where the AABB would wrongly hit;
    tolerance is screen-constant across two zoom levels.
- **Rust**: one `cutplan` test that a rect committed with a 30° rotation plans an outline whose
  vertices are the rotated corners — the claim this spec leans on, pinned rather than assumed.
- **Playwright** (`e2e/smoke.spec.ts`, fake backend): ⌘-wheel changes the status-bar zoom; dragging
  a corner handle issues one `commit_transform` whose matrix scales about the opposite corner;
  a marquee over two shapes selects both. Canvas pixels stay unread, as they are today.
- **Manual** (`MANUAL-CHECKLIST.md`): pinch and two-finger pan on a Mac trackpad; crisp strokes on
  Retina; rotate a rect 30° then cut it on the Cameo and measure; pan/zoom stays smooth on a large
  imported SVG (target: 60 fps on a 5 000-path file — a generator script is committed rather than
  the file itself, so the number can be re-measured without a megabyte of SVG in history).

## Definition of done

Open a large SVG → it fits the window, sharp on Retina → pinch into a letter and pan around at
60 fps → marquee a word, drag a corner with Shift to scale it proportionally, rotate it 15° at a
time → one undo reverts each gesture → cut it, and the cut matches the screen.

## Out of scope

Next specs, in this order, since each is independently useful and this one is already full:

- Snapping and smart guides (to artboard, to other shapes' edges and centres).
- Align/distribute, group/ungroup on canvas.
- The ∠ field in the properties panel (rotation of a multi-selection has no single value; it
  needs its own decision).
- Weed lines and weeding border.
- Hit-testing on the outline rather than the box; persisting the view per project.

## Settled after review

- **A plain mouse wheel pans, as a trackpad does** (decided 2026-10-06). Silhouette Studio and
  Inkscape zoom on the wheel and Figma pans; one rule for both devices beats matching either
  tool. It is one branch in `viewport.ts` if Windows/Linux mouse users push back.
