<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Viewport and selection handles — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the design canvas zoom and pan, Retina-sharp rendering, scale/rotate/move handles, marquee selection and rotation-aware hit-testing, so the editor can make the speed-and-clarity case against Silhouette Studio.

**Architecture:** One `View` (uniform scale + translation, world mm → CSS px) lives in React state inside a new `useCanvasInteraction` hook, never crosses IPC and never enters undo. Every pointer event is converted to world mm once, at the top of its handler. Pure modules carry all the maths — `render/affine.ts`, `interaction/viewport.ts`, `selectionBox.ts`, `gesture.ts`, `marquee.ts` — and each gesture reduces to one world-space `Matrix` that both the optimistic preview and the single `commit_transform` use. The renderer strokes in screen space (constant line width), caches `Path2D` per node, and draws the selection box, handles and marquee as an overlay.

**Tech Stack:** React + TypeScript (`apps/desktop/ui`), vitest, Playwright; one Rust test in `crates/cutplan`. **No new dependencies** in either language.

**Spec:** `docs/superpowers/specs/2026-10-06-viewport-and-handles-design.md` — read it first. It records why the view is not document state, the gesture conventions, why there is no flip-by-drag, and what is deliberately out of scope.

> **Review on #298 overturned four details after this plan was written. The task bodies below
> still state the originals and are deliberately left as they were** — this file is a record of
> the instructions the implementation was given, not a description of what shipped. The shipped
> behaviour is:
>
> 1. **Hit-test margins come from the inverse's rows** (`tol * hypot(inv[0], inv[2])`,
>    `tol * hypot(inv[1], inv[3])`), not `tol / axisLengths(world)` as Task 3 says. The two agree
>    only when a node's axes stay perpendicular in world; a rotated node under a non-uniformly
>    scaled Group is sheared (Copilot). See `apps/desktop/ui/src/render/hittest.ts`.
> 2. **The cursor readout is held in screen px and converted per render**, not stored in world mm
>    at each pointer move as Task 9 says, so a pan or zoom under a still pointer updates it
>    (Copilot).
> 3. **One transform commit is on the wire at a time.** A press while a commit is unanswered
>    selects but does not drag, since a matrix built on that commit's preview lands about the wrong
>    anchor if the commit is refused. Between the answer and the re-render, gestures start from
>    the preview (`gestureScene` in `interaction/transform.ts`) rather than the committed scene,
>    which still holds the old geometry until the snapshot lands (CodeRabbit, two rounds).
> 4. **`commit` reports `"applied"` or `"refused"`, not a boolean through `run`,** and the hook owns
>    all scene drawing through `repaint()`, which App's effect calls. A commit that landed but
>    whose refresh failed used to revert the preview onto geometry the backend no longer had, and
>    App's effect repainted the committed scene over a live drag on a mid-gesture pinch. Space and
>    the zoom chords are no longer ignored on a focused button, which every toolbar click leaves
>    behind (stage-2 review agents).
>
> The rename of `then` to `compose` *was* applied to the task bodies, because it was found before
> any reviewer saw them and the original name breaks the module outright.

## Global Constraints

**Reading the code blocks in this plan:** a block is the complete text of what it introduces unless the surrounding step says it replaces or adds to part of an existing file, in which case the step names the file and the lines it replaces.

- **SPDX header on every file** — `// SPDX-License-Identifier: GPL-3.0-or-later`. New files need one.
- **`cargo test --workspace --locked` is the gate.** No dependency is added, so `Cargo.lock` must not change.
- **`ui/dist` is committed.** Every task that edits `apps/desktop/ui/src` (Tasks 2–9) ends with `npm --prefix apps/desktop/ui run build` and commits `apps/desktop/ui/dist` in the same commit — CI rebuilds and fails on a stale bundle.
- **`apps/desktop/ipc-inventory.json` must not change.** No command, argument name or `#[tauri::command]` attribute moves; `commit_transform(ids, m)` already takes any affine.
- **The e2e fake mirrors the real backend** (`CLAUDE.md`, *Desktop app*). Its `commit_transform` keeps only the translation today; Task 10 makes it compose the full matrix, because a fake that drops scale and rotation passes a frontend whose preview and commit disagree.
- **Comments explain why, not what.** `// ponytail:` marks a deliberate simplification with its ceiling and upgrade path; this plan specifies five.
- **Vocabulary:** `CONTEXT.md` gains nothing — view, handle and marquee are UI terms, not domain ones. Do not call a Node a "layer" or an "object" in new code; it is a Node, or a shape when it is one.
- **Out of scope, and must not creep in:** snapping and smart guides, align/distribute, group/ungroup on canvas, the ∠ properties field, weed lines, outline-accurate hit-testing, persisting the view, flip/mirror. Each is named in the spec's *Out of scope* as its own follow-up.

## File Structure

| File | Responsibility after this change |
|---|---|
| `crates/cutplan/src/passes.rs` | Gains one test pinning that a rotation committed through `transform_nodes` plans as the rotated outline. |
| `apps/desktop/ui/src/render/affine.ts` | **New.** `compose`, `apply`, `invert`, `translate`, `scaleAbout`, `rotateAbout`, `axisLengths`, `isIdentity`, `transformBounds`. Replaces `App.tsx`'s private `composeThen`/`applyAffine`. |
| `apps/desktop/ui/src/render/hittest.ts` | `SceneNode.local`; hit-testing in the node's own frame with a world-mm tolerance. |
| `apps/desktop/ui/src/render/Renderer.ts` | Interface gains `resize`, `setView`, `setOverlay`; owns the `Overlay` type. |
| `apps/desktop/ui/src/render/Canvas2DRenderer.ts` | DPR backing store, view transform, screen-space strokes, `Path2D` cache, box/handles/marquee overlay. |
| `apps/desktop/ui/src/interaction/transform.ts` | `applyOptimistic` previews any affine; re-exports `Pt` from `affine.ts`. |
| `apps/desktop/ui/src/interaction/viewport.ts` | **New.** `View`, fit, zoom about a point, pan, conversions, zoom %. |
| `apps/desktop/ui/src/interaction/selectionBox.ts` | **New.** Oriented (one node) or union (several) box, handle positions, `handleAt`. |
| `apps/desktop/ui/src/interaction/gesture.ts` | **New.** `(kind, box, start, cur, modifiers) → Matrix` for move, scale and rotate. |
| `apps/desktop/ui/src/interaction/marquee.ts` | **New.** Band rectangle, band hits, selection-set rules (`toggleId` moves here from `App.tsx`). |
| `apps/desktop/ui/src/interaction/useCanvasInteraction.ts` | **New.** Owns the view, canvas sizing, wheel/keys/pointer events; drives the renderer during gestures. |
| `apps/desktop/ui/src/App.tsx` | Builds `local` into the scene; wires the hook; loses the mouse-drag code (≈60 lines out). |
| `apps/desktop/ui/src/panels/StatusBar.tsx` | Zoom % and cursor position in mm. |
| `apps/desktop/ui/src/panels/LayersPanel.tsx` | `data-selected` on each row, so e2e can see a selection. |
| `apps/desktop/ui/e2e/smoke.spec.ts` | Fake composes the full matrix and records commits; three new tests. |
| `apps/desktop/ui/scripts/perf-svg.mjs` | **New.** Generates the 5 000-path SVG for the manual frame-rate check. |
| `apps/desktop/MANUAL-CHECKLIST.md` | A *Viewport and handles* section. |

---

### Task 1: Pin that a committed rotation reaches the blade

The whole plan rests on `commit_transform` + `plan_passes` already handling rotation. Prove it before building on it.

**Files:**
- Modify: `crates/cutplan/src/passes.rs` (test module, after `plans_group_by_stroke_rgba_with_single_traversal_transforms`)

**Interfaces:** Consumes `document::commands::transform_nodes`, `plan_passes`. Produces nothing new.

- [ ] **Step 1: Write the test**

```rust
    /// The viewport spec (2026-10-06) leans on this: a rotation committed from a canvas handle
    /// reaches the blade as the rotated outline, not as the shape's axis-aligned box. Rotation
    /// enters through `transform_nodes` exactly as `commit_transform` sends it.
    #[test]
    fn plans_a_rotated_rect_as_its_rotated_corners() {
        let mut ed = Editor::new();
        let root = ed.doc.root;
        let id = ed.doc.ids.next();
        let node = Node::shape(id, ShapeKind::Rect { w: 10.0, h: 4.0 });
        ed.commit(Delta(vec![NodeOp::Add { parent: root, node, index: usize::MAX }]));

        let (s, c) = 30f64.to_radians().sin_cos();
        let rotate = Affine([c, s, -s, c, 0.0, 0.0]);
        let d = document::commands::transform_nodes(&ed.doc, &[id], rotate).unwrap();
        ed.commit(d);

        let planned = plan_passes(&ed.doc).unwrap();
        let points = &planned.passes[0].shapes[0].polylines[0];
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 4.0), (0.0, 4.0)] {
            let (ex, ey) = rotate.apply(x, y);
            assert!(
                points.iter().any(|p| (p.x - ex).abs() < 1e-9 && (p.y - ey).abs() < 1e-9),
                "corner ({x}, {y}) should plan at ({ex}, {ey}); planned {points:?}"
            );
        }
    }
```

- [ ] **Step 2: Run it — expect PASS**

Run: `cargo test -p cutplan plans_a_rotated_rect --locked`
Expected: PASS. This is a characterization test: it pins existing behaviour. **If it fails, stop** — the spec's claim that this is UI-only work is wrong, and the plan needs revisiting before Task 2.

- [ ] **Step 3: Commit**

```bash
git add crates/cutplan/src/passes.rs
git commit -m "Pin that a committed rotation plans as the rotated outline, since canvas handles will rely on it"
```

---

### Task 2: One affine module, and a preview that takes any matrix

Behaviour-preserving for the app as it stands (the canvas still only translates), but every later task needs these.

**Files:**
- Create: `apps/desktop/ui/src/render/affine.ts`, `apps/desktop/ui/src/render/affine.test.ts`
- Modify: `apps/desktop/ui/src/render/hittest.ts` (type only: add `local`), `apps/desktop/ui/src/interaction/transform.ts`, `apps/desktop/ui/src/interaction/transform.test.ts`, `apps/desktop/ui/src/App.tsx`

**Interfaces:**
- Produces: `compose`, `apply`, `invert`, `translate`, `scaleAbout`, `rotateAbout`, `axisLengths`, `isIdentity`, `transformBounds`, `IDENTITY`, `Pt` from `render/affine.ts`; `SceneNode.local?: Bounds`; `applyOptimistic` for any affine.

- [ ] **Step 1: Write the failing tests**

`apps/desktop/ui/src/render/affine.test.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { apply, IDENTITY, invert, isIdentity, rotateAbout, scaleAbout, compose, transformBounds, translate, type Pt } from "./affine";

const close = (a: Pt, b: Pt) => {
  expect(a.x).toBeCloseTo(b.x, 9);
  expect(a.y).toBeCloseTo(b.y, 9);
};

describe("affine", () => {
  it("then applies the left matrix first, as the Rust Affine::then does", () => {
    // Scale-then-translate and translate-then-scale differ; this pins which one `then` means.
    const m = compose(scaleAbout(2, 2, { x: 0, y: 0 }), translate(10, 0));
    close(apply(m, { x: 1, y: 1 }), { x: 12, y: 2 });
  });

  it("invert undoes a rotate, a non-uniform scale and a translate", () => {
    const m = compose(compose(rotateAbout(0.7, { x: 3, y: 4 }), scaleAbout(2, 0.5, { x: 0, y: 0 })), translate(5, -2));
    const inv = invert(m);
    expect(inv).not.toBeNull();
    close(apply(inv!, apply(m, { x: 7, y: 11 })), { x: 7, y: 11 });
  });

  it("invert refuses a singular matrix", () => {
    expect(invert([0, 0, 0, 1, 0, 0])).toBeNull();
  });

  it("scaleAbout and rotateAbout keep their centre fixed", () => {
    close(apply(scaleAbout(3, 0.25, { x: 4, y: 9 }), { x: 4, y: 9 }), { x: 4, y: 9 });
    close(apply(rotateAbout(1.1, { x: 4, y: 9 }), { x: 4, y: 9 }), { x: 4, y: 9 });
  });

  it("rotateAbout turns +x towards +y, which is clockwise on a y-down screen", () => {
    close(apply(rotateAbout(Math.PI / 2, { x: 0, y: 0 }), { x: 1, y: 0 }), { x: 0, y: 1 });
  });

  it("transformBounds boxes every corner of a rotated rect", () => {
    const b = transformBounds(rotateAbout(Math.PI / 4, { x: 5, y: 5 }), { x: 0, y: 0, w: 10, h: 10 });
    const half = 5 * Math.SQRT2;
    expect(b.x).toBeCloseTo(5 - half, 9);
    expect(b.w).toBeCloseTo(2 * half, 9);
  });

  it("isIdentity tolerates float noise and nothing more", () => {
    expect(isIdentity(IDENTITY)).toBe(true);
    expect(isIdentity([1, 0, 0, 1, 1e-12, 0])).toBe(true);
    expect(isIdentity(translate(0.01, 0))).toBe(false);
  });
});
```

Append to `apps/desktop/ui/src/interaction/transform.test.ts`, inside the existing `describe`, and add `import { rotateAbout } from "../render/affine";` and `import type { Affine6 } from "../render/hittest";` to its imports:

```ts
  it("applyOptimistic previews a rotation through world and bounds", () => {
    const scene = {
      nodes: [{ id: 1, bounds: { x: 0, y: 0, w: 10, h: 10 }, local: { x: 0, y: 0, w: 10, h: 10 },
                world: [1, 0, 0, 1, 0, 0] as Affine6 }],
    };
    const out = applyOptimistic(scene, [1], rotateAbout(Math.PI / 4, { x: 5, y: 5 }));
    expect(out.nodes[0].bounds.w).toBeCloseTo(10 * Math.SQRT2, 9);
    expect(out.nodes[0].world![1]).toBeCloseTo(Math.SQRT1_2, 9);
  });
```

- [ ] **Step 2: Run — expect FAIL**

Run: `npm --prefix apps/desktop/ui test -- affine transform`
Expected: FAIL — `./affine` does not exist; the rotation test cannot import.

- [ ] **Step 3: Implement `render/affine.ts`**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { Affine6, Bounds } from "./hittest";

export type Pt = { x: number; y: number };

export const IDENTITY: Affine6 = [1, 0, 0, 1, 0, 0];

/** Mirrors crates/geometry/src/affine.rs's `Affine::then`: apply `self`, then `other`. The Rust
 *  order on purpose — a matrix built here crosses IPC as `commit_transform`'s `m` and is composed
 *  there by the same rule, so the two sides cannot disagree about what a gesture meant. */
export function compose(self: Affine6, other: Affine6): Affine6 {
  const [a1, b1, c1, d1, e1, f1] = self;
  const [a2, b2, c2, d2, e2, f2] = other;
  return [
    a2 * a1 + c2 * b1,
    b2 * a1 + d2 * b1,
    a2 * c1 + c2 * d1,
    b2 * c1 + d2 * d1,
    a2 * e1 + c2 * f1 + e2,
    b2 * e1 + d2 * f1 + f2,
  ];
}

export function apply(m: Affine6, p: Pt): Pt {
  const [a, b, c, d, e, f] = m;
  return { x: a * p.x + c * p.y + e, y: b * p.x + d * p.y + f };
}

/** `null` for a singular matrix: a node scaled to nothing has no inside to click or drag. */
export function invert(m: Affine6): Affine6 | null {
  const [a, b, c, d, e, f] = m;
  const det = a * d - b * c;
  if (det === 0) return null;
  const ia = d / det;
  const ib = -b / det;
  const ic = -c / det;
  const id = a / det;
  return [ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)];
}

export function translate(dx: number, dy: number): Affine6 {
  return [1, 0, 0, 1, dx, dy];
}

export function scaleAbout(sx: number, sy: number, c: Pt): Affine6 {
  return [sx, 0, 0, sy, c.x - sx * c.x, c.y - sy * c.y];
}

export function rotateAbout(rad: number, c: Pt): Affine6 {
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  return [cos, sin, -sin, cos, c.x - cos * c.x + sin * c.y, c.y - sin * c.x - cos * c.y];
}

/** How long one unit along local x, and along local y, is after `m`. Converts a world-space
 *  tolerance or minimum size into a node's own units axis by axis, under any scale or rotation. */
export function axisLengths(m: Affine6): [number, number] {
  return [Math.hypot(m[0], m[1]), Math.hypot(m[2], m[3])];
}

export function isIdentity(m: Affine6, eps = 1e-9): boolean {
  return m.every((v, i) => Math.abs(v - IDENTITY[i]) <= eps);
}

/** Axis-aligned box of `b` after `m`. All four corners, not two, so rotation is covered. */
export function transformBounds(m: Affine6, b: Bounds): Bounds {
  const corners = [
    apply(m, { x: b.x, y: b.y }),
    apply(m, { x: b.x + b.w, y: b.y }),
    apply(m, { x: b.x, y: b.y + b.h }),
    apply(m, { x: b.x + b.w, y: b.y + b.h }),
  ];
  const xs = corners.map((p) => p.x);
  const ys = corners.map((p) => p.y);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
}
```

- [ ] **Step 4: Add `local` to `SceneNode`** in `render/hittest.ts` (the function body changes in Task 3), replacing the existing `SceneNode` line:

```ts
/** `bounds` is the world-space axis-aligned box — what a marquee tests, and all a node without
 *  geometry has. `local` is the shape's own box before `world`: what a click is tested against,
 *  so a rotated shape is hit where it is drawn rather than across its whole bounding box. */
export type SceneNode = { id: number; bounds: Bounds; local?: Bounds; shape?: ShapeGeom; world?: Affine6 };
```

- [ ] **Step 5: Generalise `interaction/transform.ts`** — replace everything above `export type DeltaOp` with:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { Affine6, Scene, SceneNode } from "../render/hittest";
import { compose, transformBounds, type Pt } from "../render/affine";

export type { Pt };
export type Matrix = Affine6; // a b c d e f

export function dragMatrix(start: Pt, cur: Pt): Matrix {
  return [1, 0, 0, 1, cur.x - start.x, cur.y - start.y];
}

/** Previews `m` on the selected nodes with no round trip. Any affine, not only a translation:
 *  handles scale and rotate through here, and the preview has to be the matrix the commit sends. */
export function applyOptimistic(scene: Scene, ids: number[], m: Matrix): Scene {
  return { nodes: scene.nodes.map((n) => (ids.includes(n.id) ? transformNode(n, m) : n)) };
}

function transformNode(n: SceneNode, m: Matrix): SceneNode {
  if (n.world && n.local) {
    const world = compose(n.world, m);
    return { ...n, world, bounds: transformBounds(world, n.local) };
  }
  return { ...n, world: n.world ? compose(n.world, m) : n.world, bounds: transformBounds(m, n.bounds) };
}
```

`DeltaOp` and `reconcile` stay as they are.

- [ ] **Step 6: Point `App.tsx` at the shared module**

Delete `composeThen`, `applyAffine` and `IDENTITY_AFFINE6` (`App.tsx:83-104`). Import `{ IDENTITY, compose, transformBounds } from "./render/affine"`. Replace `buildScene` (`App.tsx:106-141`) with:

```ts
function buildScene(doc: DocSnapshot): Scene {
  const nodes: Scene["nodes"] = [];
  const walk = (id: number, parentWorld: Affine6) => {
    const n = doc.nodes[id];
    if (!n) return;
    const world = compose(n.transform, parentWorld);
    if (typeof n.kind === "object" && "Shape" in n.kind) {
      // `local` travels with the node so hit-testing and handles work in its own frame;
      // `bounds` is its axis-aligned world box, for the marquee and the properties panel.
      const local = shapeBounds(n.kind.Shape);
      nodes.push({ id: n.id, bounds: transformBounds(world, local), local, shape: shapeGeom(n.kind.Shape), world });
    } else {
      for (const child of n.children) walk(child, world);
    }
  };
  walk(doc.root, IDENTITY);
  return { nodes };
}
```

- [ ] **Step 7: Run — expect PASS, and the build**

Run: `npm --prefix apps/desktop/ui test && npm --prefix apps/desktop/ui run build`
Expected: all vitest files PASS (the four existing `transform` tests unchanged); `tsc` clean.

- [ ] **Step 8: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Share one affine module and preview any matrix, since handles will scale and rotate through the same path as drags"
```

---

### Task 3: Hit-test in the node's own frame

**Files:**
- Modify: `apps/desktop/ui/src/render/hittest.ts`, `apps/desktop/ui/src/render/hittest.test.ts`

**Interfaces:** `hitTest(scene, x, y, tol = 0)` — `x`, `y`, `tol` in world mm. Existing callers pass no `tol` and keep today's behaviour for nodes without `local`.

- [ ] **Step 1: Write the failing tests** — append inside `describe("hitTest")`, adding `import { rotateAbout, transformBounds } from "./affine";`:

```ts
  // A 10 mm square turned 45° about its centre: its axis-aligned bounds start at 5 - 5√2 ≈ -2.07.
  const turn = rotateAbout(Math.PI / 4, { x: 5, y: 5 });
  const diamond = {
    id: 7,
    bounds: transformBounds(turn, { x: 0, y: 0, w: 10, h: 10 }),
    local: { x: 0, y: 0, w: 10, h: 10 },
    world: turn,
  };

  it("tests a rotated shape in its own frame, not across its bounding box", () => {
    const scene = { nodes: [diamond] };
    expect(hitTest(scene, 5, 5)).toBe(7);
    expect(hitTest(scene, -1, -1)).toBe(null); // inside the bounds, outside the diamond
  });

  it("widens by a world-space tolerance", () => {
    const scene = { nodes: [diamond] };
    const x = 5 - 5 * Math.SQRT2 - 1; // 1 mm beyond the left vertex
    expect(hitTest(scene, x, 5, 0)).toBe(null);
    expect(hitTest(scene, x, 5, 2)).toBe(7);
  });

  it("a node scaled to nothing cannot be hit", () => {
    const flat = { id: 8, bounds: { x: 0, y: 0, w: 0, h: 0 }, local: { x: 0, y: 0, w: 10, h: 10 },
                   world: [0, 0, 0, 0, 0, 0] as [number, number, number, number, number, number] };
    expect(hitTest({ nodes: [flat] }, 0, 0, 5)).toBe(null);
  });
```

- [ ] **Step 2: Run — expect FAIL**

Run: `npm --prefix apps/desktop/ui test -- hittest`
Expected: FAIL — `(-1, -1)` hits the diamond (AABB test); the tolerance argument is ignored.

- [ ] **Step 3: Implement** — replace `hitTest` in `render/hittest.ts`, adding `import { apply, axisLengths, invert } from "./affine";`:

```ts
/** Topmost node under (x, y). Everything is world mm, `tol` included: the caller divides a CSS-px
 *  constant by the view scale, so a thin line is as easy to click zoomed out as zoomed in. */
export function hitTest(scene: Scene, x: number, y: number, tol = 0): number | null {
  for (let i = scene.nodes.length - 1; i >= 0; i--) {
    // topmost last
    if (contains(scene.nodes[i], x, y, tol)) return scene.nodes[i].id;
  }
  return null;
}

// ponytail: tests the shape's box, not its outline, so clicking the hole of an "O" selects it.
// Ceiling: fine for primitives and solid shapes. Upgrade: `isPointInStroke` on the renderer's
// cached Path2D with a widened lineWidth, once someone reports it.
function contains(n: SceneNode, x: number, y: number, tol: number): boolean {
  if (n.local && n.world) {
    const inv = invert(n.world);
    if (!inv) return false;
    const q = apply(inv, { x, y });
    const [lx, ly] = axisLengths(n.world);
    const tx = lx > 0 ? tol / lx : 0;
    const ty = ly > 0 ? tol / ly : 0;
    const b = n.local;
    return q.x >= b.x - tx && q.x <= b.x + b.w + tx && q.y >= b.y - ty && q.y <= b.y + b.h + ty;
  }
  const b = n.bounds;
  return x >= b.x - tol && x <= b.x + b.w + tol && y >= b.y - tol && y <= b.y + b.h + tol;
}
```

- [ ] **Step 4: Run — expect PASS**, then build

Run: `npm --prefix apps/desktop/ui test -- hittest && npm --prefix apps/desktop/ui run build`

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Hit-test a shape in its own frame, since a rotated shape's bounding box is up to twice its area"
```

---

### Task 4: The viewport maths

**Files:**
- Create: `apps/desktop/ui/src/interaction/viewport.ts`, `apps/desktop/ui/src/interaction/viewport.test.ts`

**Interfaces:** Produces `View`, `Size`, `CSS_PX_PER_MM`, `MAX_SCALE`, `FIT_MARGIN_PX`, `ZOOM_STEP`, `IDENTITY_VIEW`, `fitView`, `minScaleFor`, `zoomAt`, `panBy`, `screenToWorld`, `worldToScreen`, `viewMatrix`, `zoomPercent`, `wheelFactor`. Task 9 consumes all of them.

- [ ] **Step 1: Write the failing tests** — `viewport.test.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import {
  CSS_PX_PER_MM, FIT_MARGIN_PX, MAX_SCALE, fitView, minScaleFor, panBy, screenToWorld,
  wheelFactor, worldToScreen, zoomAt, zoomPercent,
} from "./viewport";
import type { Pt } from "../render/affine";

const close = (a: Pt, b: Pt) => {
  expect(a.x).toBeCloseTo(b.x, 9);
  expect(a.y).toBeCloseTo(b.y, 9);
};

describe("viewport", () => {
  const artboard = { x: 0, y: 0, w: 300, h: 600 };
  const size = { w: 800, h: 600 };

  it("fit centres the artboard inside the margin", () => {
    const v = fitView(artboard, size);
    expect(v.scale).toBeCloseTo((600 - 2 * FIT_MARGIN_PX) / 600, 9); // height-bound here
    const tl = worldToScreen(v, { x: 0, y: 0 });
    const br = worldToScreen(v, { x: 300, y: 600 });
    expect(tl.y).toBeCloseTo(FIT_MARGIN_PX, 9);
    expect(br.y).toBeCloseTo(600 - FIT_MARGIN_PX, 9);
    expect((tl.x + br.x) / 2).toBeCloseTo(400, 9);
  });

  it("fit honours an artboard that does not start at the origin", () => {
    const v = fitView({ x: 50, y: -20, w: 100, h: 100 }, { w: 400, h: 400 });
    close(worldToScreen(v, { x: 100, y: 30 }), { x: 200, y: 200 });
  });

  it("screenToWorld inverts worldToScreen", () => {
    const v = { scale: 2.5, tx: -40, ty: 13 };
    close(screenToWorld(v, worldToScreen(v, { x: 7, y: -3 })), { x: 7, y: -3 });
  });

  it("zoomAt keeps the world point under the cursor where it was", () => {
    const v = fitView(artboard, size);
    const cursor = { x: 123, y: 456 };
    const z = zoomAt(v, cursor, 3, 0);
    expect(z.scale).toBeCloseTo(v.scale * 3, 9);
    close(screenToWorld(z, cursor), screenToWorld(v, cursor));
  });

  it("zoomAt clamps at both ends and still pins the cursor", () => {
    const v = { scale: 1, tx: 0, ty: 0 };
    expect(zoomAt(v, { x: 10, y: 10 }, 1e6, 0.5).scale).toBe(MAX_SCALE);
    const out = zoomAt(v, { x: 10, y: 10 }, 1e-6, 0.5);
    expect(out.scale).toBe(0.5);
    close(screenToWorld(out, { x: 10, y: 10 }), screenToWorld(v, { x: 10, y: 10 }));
  });

  it("the zoom floor is a quarter of fit", () => {
    expect(minScaleFor(artboard, size)).toBeCloseTo(fitView(artboard, size).scale / 4, 9);
  });

  it("a wheel delta and its negation cancel, and scrolling up zooms in", () => {
    expect(wheelFactor(37) * wheelFactor(-37)).toBeCloseTo(1, 12);
    expect(wheelFactor(-100)).toBeGreaterThan(1);
  });

  it("100% is actual size", () => {
    expect(zoomPercent({ scale: CSS_PX_PER_MM, tx: 0, ty: 0 })).toBe(100);
  });

  it("panBy moves the view and leaves the scale alone", () => {
    expect(panBy({ scale: 2, tx: 1, ty: 1 }, 10, -5)).toEqual({ scale: 2, tx: 11, ty: -4 });
  });
});
```

- [ ] **Step 2: Run — expect FAIL** (`./viewport` missing): `npm --prefix apps/desktop/ui test -- viewport`

- [ ] **Step 3: Implement `viewport.ts`**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { Affine6, Bounds } from "../render/hittest";
import type { Pt } from "../render/affine";

/** screen = world · scale + (tx, ty). World is document mm; screen is CSS px from the canvas's
 *  top-left. Uniform scale only: nobody wants a squashed view of a cut. */
export type View = { scale: number; tx: number; ty: number };
export type Size = { w: number; h: number };

/** "100%" is actual size. CSS fixes 96 px to the inch; the document is in millimetres. */
export const CSS_PX_PER_MM = 96 / 25.4;
export const MAX_SCALE = 64 * CSS_PX_PER_MM; // 6400%
export const FIT_MARGIN_PX = 24;
export const ZOOM_STEP = 1.25;
/** Pinch reports a few px per event and a mouse notch about 100. An exponential keeps both
 *  proportional, and zooming in then out by the same delta lands exactly where it started. */
export const WHEEL_ZOOM_RATE = 0.01;

export const IDENTITY_VIEW: View = { scale: 1, tx: 0, ty: 0 };

export function fitView(artboard: Bounds, size: Size): View {
  if (artboard.w <= 0 || artboard.h <= 0) {
    return {
      scale: CSS_PX_PER_MM,
      tx: FIT_MARGIN_PX - artboard.x * CSS_PX_PER_MM,
      ty: FIT_MARGIN_PX - artboard.y * CSS_PX_PER_MM,
    };
  }
  const availW = Math.max(size.w - 2 * FIT_MARGIN_PX, 1);
  const availH = Math.max(size.h - 2 * FIT_MARGIN_PX, 1);
  const scale = Math.min(availW / artboard.w, availH / artboard.h);
  return {
    scale,
    tx: (size.w - artboard.w * scale) / 2 - artboard.x * scale,
    ty: (size.h - artboard.h * scale) / 2 - artboard.y * scale,
  };
}

/** Far enough out to see the artboard small with room around it, and no further: past that the
 *  canvas is empty space and the way back is long. */
export function minScaleFor(artboard: Bounds, size: Size): number {
  return Math.min(fitView(artboard, size).scale / 4, MAX_SCALE);
}

export function zoomAt(v: View, screen: Pt, factor: number, minScale: number): View {
  const scale = Math.min(Math.max(v.scale * factor, minScale), MAX_SCALE);
  const k = scale / v.scale;
  return { scale, tx: screen.x - (screen.x - v.tx) * k, ty: screen.y - (screen.y - v.ty) * k };
}

export function panBy(v: View, dx: number, dy: number): View {
  return { scale: v.scale, tx: v.tx + dx, ty: v.ty + dy };
}

export function worldToScreen(v: View, p: Pt): Pt {
  return { x: p.x * v.scale + v.tx, y: p.y * v.scale + v.ty };
}

export function screenToWorld(v: View, p: Pt): Pt {
  return { x: (p.x - v.tx) / v.scale, y: (p.y - v.ty) / v.scale };
}

export function viewMatrix(v: View): Affine6 {
  return [v.scale, 0, 0, v.scale, v.tx, v.ty];
}

export function zoomPercent(v: View): number {
  return Math.round((v.scale / CSS_PX_PER_MM) * 100);
}

export function wheelFactor(deltaY: number): number {
  return Math.exp(-deltaY * WHEEL_ZOOM_RATE);
}
```

- [ ] **Step 4: Run — expect PASS**, then build: `npm --prefix apps/desktop/ui test -- viewport && npm --prefix apps/desktop/ui run build`

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Add the viewport maths as a pure module, since zoom about the cursor is easy to get subtly wrong"
```

---

### Task 5: The selection box and its handles

**Files:**
- Create: `apps/desktop/ui/src/interaction/selectionBox.ts`, `apps/desktop/ui/src/interaction/selectionBox.test.ts`

**Interfaces:** Produces `Box`, `ScaleHandle`, `HandleKind`, `HANDLE_UNIT`, `SCALE_HANDLES`, `selectionBox`, `handleLocal`, `handleWorld`, `boxCorners`, `boxCenter`, `handleAt`. Consumed by Task 6 (gesture), Task 8 (renderer overlay), Task 9 (hook).

- [ ] **Step 1: Write the failing tests** — `selectionBox.test.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { apply, IDENTITY, rotateAbout, transformBounds, translate, type Pt } from "../render/affine";
import { handleAt, handleWorld, selectionBox } from "./selectionBox";

const close = (a: Pt, b: Pt) => {
  expect(a.x).toBeCloseTo(b.x, 9);
  expect(a.y).toBeCloseTo(b.y, 9);
};

const r30 = rotateAbout(Math.PI / 6, { x: 0, y: 0 });
const rotated = {
  id: 1,
  local: { x: 0, y: 0, w: 10, h: 4 },
  world: r30,
  bounds: transformBounds(r30, { x: 0, y: 0, w: 10, h: 4 }),
};

describe("selectionBox", () => {
  it("one node's box is its own oriented box, so handles sit on its edges", () => {
    const box = selectionBox({ nodes: [rotated] }, [1])!;
    close(handleWorld(box, "ne"), apply(r30, { x: 10, y: 0 }));
    close(handleWorld(box, "s"), apply(r30, { x: 5, y: 4 }));
  });

  it("a path whose local box does not start at 0,0 still gets handles on it", () => {
    const n = { id: 2, local: { x: 3, y: 5, w: 2, h: 2 }, world: IDENTITY, bounds: { x: 3, y: 5, w: 2, h: 2 } };
    close(handleWorld(selectionBox({ nodes: [n] }, [2])!, "nw"), { x: 3, y: 5 });
  });

  it("several nodes share their axis-aligned union box", () => {
    const a = { id: 1, bounds: { x: 0, y: 0, w: 2, h: 2 } };
    const b = { id: 2, bounds: { x: 5, y: 3, w: 1, h: 4 } };
    const box = selectionBox({ nodes: [a, b] }, [1, 2])!;
    close(handleWorld(box, "nw"), { x: 0, y: 0 });
    close(handleWorld(box, "se"), { x: 6, y: 7 });
  });

  it("no selection, no box", () => {
    expect(selectionBox({ nodes: [rotated] }, [])).toBeNull();
  });
});

describe("handleAt", () => {
  const box = { frame: translate(10, 10), w: 20, h: 10 };

  it("finds handles first, then the inside, then the rotate zone", () => {
    expect(handleAt(box, { x: 30.5, y: 20.5 }, 1, 3)).toBe("se");
    expect(handleAt(box, { x: 20, y: 15 }, 1, 3)).toBe("move");
    expect(handleAt(box, { x: 32, y: 22 }, 1, 3)).toBe("rotate");
    expect(handleAt(box, { x: 50, y: 50 }, 1, 3)).toBeNull();
  });

  it("finds a rotated box's handles where they are drawn", () => {
    const b = selectionBox({ nodes: [rotated] }, [1])!;
    expect(handleAt(b, apply(r30, { x: 10, y: 4 }), 0.5, 2)).toBe("se");
  });

  it("a zero-height box (a straight path) still has an inside to drag", () => {
    const line = { frame: translate(0, 0), w: 10, h: 0 };
    expect(handleAt(line, { x: 3, y: 0 }, 0.5, 1)).toBe("move");
  });
});
```

- [ ] **Step 2: Run — expect FAIL**: `npm --prefix apps/desktop/ui test -- selectionBox`

- [ ] **Step 3: Implement `selectionBox.ts`**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { Affine6, Scene } from "../render/hittest";
import { apply, invert, compose, translate, type Pt } from "../render/affine";

/** The local rectangle [0, w] × [0, h], placed in world by `frame`. Gestures work in this frame,
 *  which is what lets one scale rule serve a rotated single node and an axis-aligned group alike. */
export type Box = { frame: Affine6; w: number; h: number };
export type ScaleHandle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
export type HandleKind = ScaleHandle | "rotate" | "move";

/** Each handle's place on the box, as fractions of (w, h). */
export const HANDLE_UNIT: Record<ScaleHandle, Pt> = {
  nw: { x: 0, y: 0 }, n: { x: 0.5, y: 0 }, ne: { x: 1, y: 0 }, e: { x: 1, y: 0.5 },
  se: { x: 1, y: 1 }, s: { x: 0.5, y: 1 }, sw: { x: 0, y: 1 }, w: { x: 0, y: 0.5 },
};
export const SCALE_HANDLES = Object.keys(HANDLE_UNIT) as ScaleHandle[];

// Float slack on the inside test, so a zero-height box (a straight path) keeps an inside.
const INSIDE_EPS = 1e-9;

/** One node: its oriented box, so a rotated rect keeps handles on its edges. Several: the
 *  axis-aligned union of their world bounds — there is no shared orientation to keep. */
export function selectionBox(scene: Scene, ids: number[]): Box | null {
  const nodes = scene.nodes.filter((n) => ids.includes(n.id));
  if (nodes.length === 0) return null;
  const only = nodes.length === 1 ? nodes[0] : null;
  if (only?.local && only.world) {
    return { frame: compose(translate(only.local.x, only.local.y), only.world), w: only.local.w, h: only.local.h };
  }
  const x = Math.min(...nodes.map((n) => n.bounds.x));
  const y = Math.min(...nodes.map((n) => n.bounds.y));
  const r = Math.max(...nodes.map((n) => n.bounds.x + n.bounds.w));
  const b = Math.max(...nodes.map((n) => n.bounds.y + n.bounds.h));
  return { frame: translate(x, y), w: r - x, h: b - y };
}

export function handleLocal(box: Box, h: ScaleHandle): Pt {
  return { x: HANDLE_UNIT[h].x * box.w, y: HANDLE_UNIT[h].y * box.h };
}

export function handleWorld(box: Box, h: ScaleHandle): Pt {
  return apply(box.frame, handleLocal(box, h));
}

/** World corners in drawing order: nw, ne, se, sw. */
export function boxCorners(box: Box): Pt[] {
  return (["nw", "ne", "se", "sw"] as const).map((h) => handleWorld(box, h));
}

export function boxCenter(box: Box): Pt {
  return apply(box.frame, { x: box.w / 2, y: box.h / 2 });
}

/** Which part of the box is under `p`. Tolerances are world mm — the caller divides its CSS-px
 *  sizes by the view scale. Handles win over the inside so a small box can still be scaled, and
 *  the rotate zone is only outside, where a press would otherwise do nothing. */
export function handleAt(box: Box, p: Pt, handleTol: number, rotateTol: number): HandleKind | null {
  for (const h of SCALE_HANDLES) {
    if (dist(handleWorld(box, h), p) <= handleTol) return h;
  }
  if (inside(box, p)) return "move";
  if (boxCorners(box).some((c) => dist(c, p) <= rotateTol)) return "rotate";
  return null;
}

function inside(box: Box, p: Pt): boolean {
  const inv = invert(box.frame);
  if (!inv) return false;
  const q = apply(inv, p);
  return q.x >= -INSIDE_EPS && q.x <= box.w + INSIDE_EPS && q.y >= -INSIDE_EPS && q.y <= box.h + INSIDE_EPS;
}

function dist(a: Pt, b: Pt): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}
```

- [ ] **Step 4: Run — expect PASS**, then build: `npm --prefix apps/desktop/ui test -- selectionBox && npm --prefix apps/desktop/ui run build`

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Give a selection an oriented box and handles, since a rotated shape's handles belong on its edges"
```

---

### Task 6: Gestures become one matrix

**Files:**
- Create: `apps/desktop/ui/src/interaction/gesture.ts`, `apps/desktop/ui/src/interaction/gesture.test.ts`

**Interfaces:** Produces `Modifiers`, `MIN_SIZE_MM`, `ROTATE_SNAP_RAD`, `gestureMatrix(kind, box, start, cur, mods): Matrix`. Start and current points are world mm.

- [ ] **Step 1: Write the failing tests** — `gesture.test.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { apply, isIdentity, rotateAbout, translate, type Pt } from "../render/affine";
import { gestureMatrix, MIN_SIZE_MM } from "./gesture";
import { boxCenter, type Box } from "./selectionBox";

const close = (a: Pt, b: Pt) => {
  expect(a.x).toBeCloseTo(b.x, 9);
  expect(a.y).toBeCloseTo(b.y, 9);
};
const none = { shift: false, alt: false };
// 40 × 20 mm, top-left at (10, 20): se corner at (50, 40), centre at (30, 30).
const box: Box = { frame: translate(10, 20), w: 40, h: 20 };

describe("move", () => {
  it("translates by the drag", () => {
    expect(gestureMatrix("move", box, { x: 0, y: 0 }, { x: 3, y: -4 }, none)).toEqual(translate(3, -4));
  });
  it("Shift locks to the longer axis", () => {
    expect(gestureMatrix("move", box, { x: 0, y: 0 }, { x: 3, y: -4 }, { shift: true, alt: false })).toEqual(translate(0, -4));
  });
  it("a press without movement is the identity, which the caller does not commit", () => {
    expect(isIdentity(gestureMatrix("move", box, { x: 5, y: 5 }, { x: 5, y: 5 }, none))).toBe(true);
  });
});

describe("scale", () => {
  it("a corner scales about the opposite corner", () => {
    const m = gestureMatrix("se", box, { x: 50, y: 40 }, { x: 90, y: 60 }, none);
    close(apply(m, { x: 10, y: 20 }), { x: 10, y: 20 });
    close(apply(m, { x: 50, y: 40 }), { x: 90, y: 60 });
  });
  it("an edge scales one axis only", () => {
    const m = gestureMatrix("e", box, { x: 50, y: 30 }, { x: 70, y: 999 }, none);
    close(apply(m, { x: 50, y: 40 }), { x: 70, y: 40 });
  });
  it("is measured from the handle, not from where the press landed", () => {
    // Grabbing 1 mm inside the corner must not change the factor.
    const m = gestureMatrix("se", box, { x: 49, y: 39 }, { x: 89, y: 59 }, none);
    close(apply(m, { x: 50, y: 40 }), { x: 90, y: 60 });
  });
  it("Alt scales about the centre", () => {
    const m = gestureMatrix("se", box, { x: 50, y: 40 }, { x: 60, y: 45 }, { shift: false, alt: true });
    close(apply(m, boxCenter(box)), boxCenter(box));
    close(apply(m, { x: 50, y: 40 }), { x: 60, y: 45 });
  });
  it("Shift keeps the proportions, following the axis that moved most", () => {
    const m = gestureMatrix("se", box, { x: 50, y: 40 }, { x: 90, y: 44 }, { shift: true, alt: false });
    close(apply(m, { x: 50, y: 40 }), { x: 90, y: 60 }); // ×2 both ways
  });
  it("stops at the minimum size rather than flipping", () => {
    const m = gestureMatrix("se", box, { x: 50, y: 40 }, { x: -100, y: 40 }, none);
    const ne = apply(m, { x: 50, y: 20 });
    expect(ne.x - 10).toBeCloseTo(MIN_SIZE_MM, 9);
  });
  it("scales a rotated box along its own axes", () => {
    const r30 = rotateAbout(Math.PI / 6, { x: 0, y: 0 });
    const rotated: Box = { frame: r30, w: 10, h: 4 };
    const grab = apply(r30, { x: 10, y: 2 });
    const along = { x: grab.x + 10 * Math.cos(Math.PI / 6), y: grab.y + 10 * Math.sin(Math.PI / 6) };
    const m = gestureMatrix("e", rotated, grab, along, none);
    close(apply(m, apply(r30, { x: 10, y: 0 })), apply(r30, { x: 20, y: 0 }));
    close(apply(m, apply(r30, { x: 0, y: 4 })), apply(r30, { x: 0, y: 4 }));
  });
});

describe("rotate", () => {
  it("turns about the box centre by the angle swept", () => {
    const c = boxCenter(box);
    const m = gestureMatrix("rotate", box, { x: c.x + 10, y: c.y }, { x: c.x, y: c.y + 10 }, none);
    close(apply(m, c), c);
    close(apply(m, { x: c.x + 10, y: c.y }), { x: c.x, y: c.y + 10 });
  });
  it("Shift snaps to 15° steps", () => {
    const c = boxCenter(box);
    const a = (50 * Math.PI) / 180;
    const m = gestureMatrix("rotate", box, { x: c.x + 10, y: c.y }, { x: c.x + 10 * Math.cos(a), y: c.y + 10 * Math.sin(a) }, { shift: true, alt: false });
    const b = (45 * Math.PI) / 180;
    close(apply(m, { x: c.x + 10, y: c.y }), { x: c.x + 10 * Math.cos(b), y: c.y + 10 * Math.sin(b) });
  });
});
```

- [ ] **Step 2: Run — expect FAIL**: `npm --prefix apps/desktop/ui test -- gesture`

- [ ] **Step 3: Implement `gesture.ts`**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { apply, axisLengths, IDENTITY, invert, rotateAbout, scaleAbout, compose, translate, type Pt } from "../render/affine";
import type { Matrix } from "./transform";
import { boxCenter, HANDLE_UNIT, handleLocal, type Box, type HandleKind, type ScaleHandle } from "./selectionBox";

export type Modifiers = { shift: boolean; alt: boolean };

/** Smallest a scale handle will take an axis, in world mm. Below it the shape is not cuttable,
 *  and crossing zero would mirror it — a mirrored cut should be a command, not an overshoot. */
export const MIN_SIZE_MM = 0.1;
export const ROTATE_SNAP_RAD = Math.PI / 12; // 15°

/** The whole gesture as one world-space matrix: the preview applies it and the commit sends it,
 *  so what the operator releases is what is saved. */
export function gestureMatrix(kind: HandleKind, box: Box, start: Pt, cur: Pt, mods: Modifiers): Matrix {
  if (kind === "move") return moveMatrix(start, cur, mods.shift);
  if (kind === "rotate") return rotateMatrix(box, start, cur, mods.shift);
  return scaleMatrix(kind, box, start, cur, mods);
}

function moveMatrix(start: Pt, cur: Pt, shift: boolean): Matrix {
  const dx = cur.x - start.x;
  const dy = cur.y - start.y;
  if (!shift) return translate(dx, dy);
  return Math.abs(dx) >= Math.abs(dy) ? translate(dx, 0) : translate(0, dy);
}

function rotateMatrix(box: Box, start: Pt, cur: Pt, shift: boolean): Matrix {
  const c = boxCenter(box);
  const swept = Math.atan2(cur.y - c.y, cur.x - c.x) - Math.atan2(start.y - c.y, start.x - c.x);
  const angle = shift ? Math.round(swept / ROTATE_SNAP_RAD) * ROTATE_SNAP_RAD : swept;
  return rotateAbout(angle, c);
}

/** Works in the box's own frame — scale there about the anchor, then carry it back to world —
 *  so a rotated box scales along its edges, and an axis-aligned one is just the special case. */
function scaleMatrix(h: ScaleHandle, box: Box, start: Pt, cur: Pt, mods: Modifiers): Matrix {
  const inv = invert(box.frame);
  if (!inv) return IDENTITY;
  const ps = apply(inv, start);
  const pc = apply(inv, cur);
  const unit = HANDLE_UNIT[h];
  const grab = handleLocal(box, h);
  const anchor = mods.alt ? { x: box.w / 2, y: box.h / 2 } : { x: (1 - unit.x) * box.w, y: (1 - unit.y) * box.h };
  const [lx, ly] = axisLengths(box.frame);
  const worldW = box.w * lx;
  const worldH = box.h * ly;
  const movesX = unit.x !== 0.5;
  const movesY = unit.y !== 0.5;
  let sx = movesX ? axisScale(grab.x, pc.x - ps.x, anchor.x, worldW) : 1;
  let sy = movesY ? axisScale(grab.y, pc.y - ps.y, anchor.y, worldH) : 1;
  if (mods.shift) {
    const s = !movesX ? sy : !movesY ? sx : Math.abs(sx - 1) >= Math.abs(sy - 1) ? sx : sy;
    const floor = Math.max(worldW > 0 ? MIN_SIZE_MM / worldW : 0, worldH > 0 ? MIN_SIZE_MM / worldH : 0);
    sx = Math.max(s, floor);
    sy = sx;
  }
  return compose(compose(inv, scaleAbout(sx, sy, anchor)), box.frame);
}

/** Factor that moves the handle at `grab` by `delta` while `anchor` stays put. Measured from the
 *  handle rather than the press point, so grabbing a handle off-centre does not skew the result. */
function axisScale(grab: number, delta: number, anchor: number, worldLen: number): number {
  const span = grab - anchor;
  if (span === 0 || worldLen === 0) return 1; // no extent along this axis to scale
  return Math.max((span + delta) / span, MIN_SIZE_MM / worldLen);
}
```

- [ ] **Step 4: Run — expect PASS**, then build: `npm --prefix apps/desktop/ui test -- gesture && npm --prefix apps/desktop/ui run build`

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Reduce move, scale and rotate to one matrix per gesture, since the preview and the commit must not disagree"
```

---

### Task 7: Marquee and selection-set rules

**Files:**
- Create: `apps/desktop/ui/src/interaction/marquee.ts`, `apps/desktop/ui/src/interaction/marquee.test.ts`
- Modify: `apps/desktop/ui/src/App.tsx` (delete the local `toggleId`, import it from here)

**Interfaces:** Produces `normalizeRect`, `marqueeHits`, `marqueeSelection`, `toggleId`.

- [ ] **Step 1: Write the failing tests** — `marquee.test.ts`:

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { marqueeHits, marqueeSelection, normalizeRect, toggleId } from "./marquee";

describe("marquee", () => {
  it("normalizes a band dragged in any direction", () => {
    expect(normalizeRect({ x: 5, y: 1 }, { x: 1, y: 4 })).toEqual({ x: 1, y: 1, w: 4, h: 3 });
  });

  it("selects what the band touches, not only what it contains", () => {
    const scene = { nodes: [
      { id: 1, bounds: { x: 0, y: 0, w: 10, h: 10 } },
      { id: 2, bounds: { x: 8, y: 8, w: 10, h: 10 } },
      { id: 3, bounds: { x: 50, y: 50, w: 1, h: 1 } },
    ] };
    expect(marqueeHits(scene, { x: 9, y: 9, w: 1, h: 1 })).toEqual([1, 2]);
  });

  it("adds with Shift, keeping order and dropping duplicates", () => {
    expect(marqueeSelection([3, 1], [1, 2], true)).toEqual([3, 1, 2]);
  });

  it("replaces without Shift", () => {
    expect(marqueeSelection([3, 1], [2], false)).toEqual([2]);
  });

  it("toggleId adds a missing id and removes a present one", () => {
    expect(toggleId([1, 2], 3)).toEqual([1, 2, 3]);
    expect(toggleId([1, 2], 1)).toEqual([2]);
  });
});
```

- [ ] **Step 2: Run — expect FAIL**: `npm --prefix apps/desktop/ui test -- marquee`

- [ ] **Step 3: Implement `marquee.ts`**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { Bounds, Scene } from "../render/hittest";
import type { Pt } from "../render/affine";

export function normalizeRect(a: Pt, b: Pt): Bounds {
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(b.x - a.x), h: Math.abs(b.y - a.y) };
}

/** Nodes whose world bounds the band touches. Touching rather than containing, because the usual
 *  target is a word of imported letters, and a band that must swallow every serif is a chore. */
export function marqueeHits(scene: Scene, band: Bounds): number[] {
  return scene.nodes
    .filter((n) => n.bounds.x <= band.x + band.w && n.bounds.x + n.bounds.w >= band.x
                && n.bounds.y <= band.y + band.h && n.bounds.y + n.bounds.h >= band.y)
    .map((n) => n.id);
}

export function marqueeSelection(prev: number[], hits: number[], additive: boolean): number[] {
  return additive ? [...prev, ...hits.filter((id) => !prev.includes(id))] : hits;
}

export function toggleId(ids: number[], id: number): number[] {
  return ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id];
}
```

In `App.tsx`, delete `function toggleId` (`App.tsx:59-61`) and add `import { toggleId } from "./interaction/marquee";`.

- [ ] **Step 4: Run — expect PASS**, then build: `npm --prefix apps/desktop/ui test && npm --prefix apps/desktop/ui run build`

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Add marquee selection rules, since handles make multi-select common and Shift-clicking every letter is a chore"
```

---

### Task 8: The renderer draws through the view

No unit tests: the renderer is Canvas2D calls, and vitest runs in `node` with no canvas. It is covered by Task 10's e2e and Task 11's manual checks; it holds no decisions, so there is nothing to unit-test.

**Files:**
- Modify: `apps/desktop/ui/src/render/Renderer.ts`, `apps/desktop/ui/src/render/Canvas2DRenderer.ts`

**Interfaces:** Produces `Overlay`; `Renderer.resize(cssW, cssH, dpr)`, `setView(m)`, `setOverlay(o)`.

- [ ] **Step 1: Replace `Renderer.ts`**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { Affine6, Bounds, Scene } from "./hittest";
import type { Box } from "../interaction/selectionBox";

export type NodeId = number;

/** What is drawn over the artwork: the selection's box and handles, and a marquee band. */
export type Overlay = { box: Box | null; marquee: Bounds | null };

export interface Renderer {
  setScene(s: Scene): void;
  markDirty(id: NodeId): void;
  setSelection(ids: NodeId[]): void;
  /** CSS size of the canvas and the display's pixel ratio; sizes the backing store. */
  resize(cssW: number, cssH: number, dpr: number): void;
  /** World mm → CSS px. */
  setView(m: Affine6): void;
  setOverlay(o: Overlay): void;
  draw(): void;
}
```

- [ ] **Step 2: Replace `Canvas2DRenderer.ts`**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { Renderer, NodeId, Overlay } from "./Renderer";
import type { Affine6, Bounds, Scene, ShapeGeom } from "./hittest";
import { apply, IDENTITY, compose, transformBounds } from "./affine";
import { boxCorners, handleWorld, SCALE_HANDLES } from "../interaction/selectionBox";

const FALLBACK_ACCENT = "#22D3EE";
const FALLBACK_BORDER = "#2E2E34";
const FALLBACK_PANEL = "#1F1F23";
const FALLBACK_TEXT = "#E7E7EA";

// CSS px, whatever the zoom.
const STROKE_PX = 1;
const SELECTED_STROKE_PX = 2;
const HANDLE_PX = 7;
const MARQUEE_ALPHA = 0.12;

export class Canvas2DRenderer implements Renderer {
  private scene: Scene = { nodes: [] };
  private selected = new Set<NodeId>();
  private artboard: Bounds | null = null;
  private view: Affine6 = IDENTITY;
  private dpr = 1;
  private overlay: Overlay = { box: null, marquee: null };
  // ponytail: invalidation only — with the current full-clear+redraw loop this is just
  // a "needs redraw" signal, not a per-node dirty rect. draw() clears it each call.
  private dirty = new Set<NodeId>();
  // Parsed geometry per node, keyed on the geometry's value: a snapshot rebuilds every ShapeGeom
  // object, so object identity would miss after each edit while an equal string still hits.
  private paths = new Map<NodeId, { key: string; path: Path2D }>();

  constructor(private readonly ctx: CanvasRenderingContext2D) {}

  setScene(s: Scene): void {
    this.scene = s;
    // Without this every deleted node's parsed path lives as long as the window.
    const live = new Set(s.nodes.map((n) => n.id));
    for (const id of this.paths.keys()) if (!live.has(id)) this.paths.delete(id);
  }

  setArtboard(b: Bounds | null): void {
    this.artboard = b;
  }

  markDirty(id: NodeId): void {
    this.dirty.add(id);
  }

  setSelection(ids: NodeId[]): void {
    this.selected = new Set(ids);
  }

  resize(cssW: number, cssH: number, dpr: number): void {
    const canvas = this.ctx.canvas;
    canvas.width = Math.max(1, Math.round(cssW * dpr));
    canvas.height = Math.max(1, Math.round(cssH * dpr));
    this.dpr = dpr;
  }

  setView(m: Affine6): void {
    this.view = m;
  }

  setOverlay(o: Overlay): void {
    this.overlay = o;
  }

  draw(): void {
    // ponytail: full clear + redraw every frame instead of patching a dirty region. With the
    // Path2D cache a frame is one matrix multiply and one stroke per node; revisit when the
    // 5 000-path check in MANUAL-CHECKLIST.md drops below 60 fps.
    const { ctx, dpr, view } = this;
    const canvas = ctx.canvas;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    // From here on, units are CSS px: the pixel ratio is applied once so Retina gets real pixels.
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const style = getComputedStyle(document.documentElement);
    const accent = style.getPropertyValue("--accent").trim() || FALLBACK_ACCENT;
    const border = style.getPropertyValue("--border").trim() || FALLBACK_BORDER;
    const panel = style.getPropertyValue("--panel").trim() || FALLBACK_PANEL;
    const text = style.getPropertyValue("--text").trim() || FALLBACK_TEXT;

    // Artboard drawn first so node outlines paint over it, not the other way around.
    if (this.artboard) {
      const a = transformBounds(view, this.artboard);
      ctx.fillStyle = panel;
      ctx.fillRect(a.x, a.y, a.w, a.h);
      ctx.strokeStyle = border;
      ctx.lineWidth = STROKE_PX;
      ctx.strokeRect(a.x, a.y, a.w, a.h);
    }

    for (const node of this.scene.nodes) {
      const selected = this.selected.has(node.id);
      ctx.strokeStyle = selected ? accent : text;
      ctx.lineWidth = selected ? SELECTED_STROKE_PX : STROKE_PX;
      if (node.shape && node.world) {
        // Geometry is carried to screen space before stroking, so the line is in CSS px whatever
        // the zoom or the node's own scale. Stroking under the node's transform scaled it too.
        const onScreen = new Path2D();
        onScreen.addPath(this.localPath(node.id, node.shape), toDOMMatrix(compose(node.world, view)));
        ctx.stroke(onScreen);
      } else {
        // Nodes without geometry (tests, mocks) keep the SP3 bounds outline.
        const b = transformBounds(view, node.bounds);
        ctx.strokeRect(b.x, b.y, b.w, b.h);
      }
    }

    this.drawOverlay(accent, panel);
    this.dirty.clear();
  }

  private drawOverlay(accent: string, panel: string): void {
    const { ctx, view } = this;
    const { box, marquee } = this.overlay;
    if (box) {
      const corners = boxCorners(box).map((p) => apply(view, p));
      ctx.beginPath();
      corners.forEach((p, i) => (i === 0 ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y)));
      ctx.closePath();
      ctx.strokeStyle = accent;
      ctx.lineWidth = STROKE_PX;
      ctx.stroke();
      ctx.fillStyle = panel;
      const half = HANDLE_PX / 2;
      for (const h of SCALE_HANDLES) {
        const p = apply(view, handleWorld(box, h));
        ctx.fillRect(p.x - half, p.y - half, HANDLE_PX, HANDLE_PX);
        ctx.strokeRect(p.x - half, p.y - half, HANDLE_PX, HANDLE_PX);
      }
    }
    if (marquee) {
      const m = transformBounds(view, marquee);
      ctx.save();
      ctx.globalAlpha = MARQUEE_ALPHA;
      ctx.fillStyle = accent;
      ctx.fillRect(m.x, m.y, m.w, m.h);
      ctx.restore();
      ctx.save();
      ctx.setLineDash([4, 3]);
      ctx.strokeStyle = accent;
      ctx.lineWidth = STROKE_PX;
      ctx.strokeRect(m.x, m.y, m.w, m.h);
      ctx.restore();
    }
  }

  private localPath(id: NodeId, shape: ShapeGeom): Path2D {
    const key = geomKey(shape);
    const hit = this.paths.get(id);
    if (hit && hit.key === key) return hit.path;
    const path = buildPath(shape);
    this.paths.set(id, { key, path });
    return path;
  }
}

function geomKey(g: ShapeGeom): string {
  if (g.t === "rect") return `r ${g.w} ${g.h}`;
  if (g.t === "ellipse") return `e ${g.rx} ${g.ry}`;
  return `p ${g.d}`;
}

function buildPath(g: ShapeGeom): Path2D {
  if (g.t === "path") return new Path2D(g.d);
  const p = new Path2D();
  if (g.t === "rect") p.rect(0, 0, g.w, g.h);
  // Canonical convention (crates/document/src/commands.rs): local space is centred at (rx, ry).
  else p.ellipse(g.rx, g.ry, g.rx, g.ry, 0, 0, Math.PI * 2);
  return p;
}

function toDOMMatrix([a, b, c, d, e, f]: Affine6): DOMMatrix {
  return new DOMMatrix([a, b, c, d, e, f]);
}
```

- [ ] **Step 3: Build — expect PASS**

Run: `npm --prefix apps/desktop/ui test && npm --prefix apps/desktop/ui run build`
Expected: PASS; `tsc` clean (`App.tsx` calls only methods that still exist). The view stays the identity and the canvas keeps its fixed 800×600 size until Task 9 wires the hook, so the app looks as it did, except that strokes no longer thicken with a node's scale.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Draw through a view with screen-constant strokes and cached paths, since every frame re-parsed every path"
```

---

### Task 9: Wire the view, the gestures and the readout

**Files:**
- Create: `apps/desktop/ui/src/interaction/useCanvasInteraction.ts`
- Modify: `apps/desktop/ui/src/App.tsx`, `apps/desktop/ui/src/panels/StatusBar.tsx`, `apps/desktop/ui/src/panels/LayersPanel.tsx`

**Interfaces:** `useCanvasInteraction(args) → { view, size, cursor, requestFit, handlers }`. The canvas publishes `data-view="scale tx ty"` and `data-testid="design-canvas"`; the status bar publishes `data-testid="status-zoom"`; each layer row publishes `data-selected`. Task 10 reads all three.

- [ ] **Step 1: Create `useCanvasInteraction.ts`**

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent, type RefObject } from "react";
import { hitTest, type Bounds, type Scene } from "../render/hittest";
import { IDENTITY, isIdentity, compose, type Pt } from "../render/affine";
import type { Canvas2DRenderer } from "../render/Canvas2DRenderer";
import { applyOptimistic, type Matrix } from "./transform";
import {
  CSS_PX_PER_MM, IDENTITY_VIEW, ZOOM_STEP, fitView, minScaleFor, panBy, screenToWorld,
  wheelFactor, zoomAt, type Size, type View,
} from "./viewport";
import { handleAt, selectionBox, type Box, type HandleKind } from "./selectionBox";
import { gestureMatrix } from "./gesture";
import { marqueeHits, marqueeSelection, normalizeRect, toggleId } from "./marquee";

// CSS px, divided by the view scale at use so they feel the same at every zoom.
const HANDLE_HIT_PX = 6;
const ROTATE_ZONE_PX = 18;
const HIT_TOL_PX = 3;
/** A press that travels less than this is a click: it clears the selection instead of banding. */
const CLICK_SLOP_PX = 3;
/** Some wheels report lines rather than pixels; one line is about one line of text. */
const LINE_PX = 16;

const CURSORS: Record<HandleKind | "pan", string> = {
  nw: "nwse-resize", se: "nwse-resize", ne: "nesw-resize", sw: "nesw-resize",
  n: "ns-resize", s: "ns-resize", e: "ew-resize", w: "ew-resize",
  rotate: "crosshair", move: "move", pan: "grab",
};

type Gesture =
  | { t: "pan"; last: Pt } // screen px
  | { t: "transform"; kind: HandleKind; box: Box; ids: number[]; start: Pt; m: Matrix } // world mm
  | { t: "marquee"; start: Pt; cur: Pt; additive: boolean }; // world mm

export type CanvasInteractionArgs = {
  canvasRef: RefObject<HTMLCanvasElement>;
  rendererRef: RefObject<Canvas2DRenderer>;
  scene: Scene;
  selected: number[];
  setSelected: (ids: number[]) => void;
  artboard: Bounds | null;
  /** Resolves false when the backend refused, so the preview can be put back. */
  commit: (ids: number[], m: Matrix) => Promise<boolean>;
};

type Handlers = {
  onPointerDown: (e: PointerEvent<HTMLCanvasElement>) => void;
  onPointerMove: (e: PointerEvent<HTMLCanvasElement>) => void;
  onPointerUp: () => void;
  onPointerCancel: () => void;
  onPointerLeave: () => void;
};

export type CanvasInteraction = {
  view: View;
  size: Size;
  cursor: Pt | null;
  requestFit: () => void;
  handlers: Handlers;
};

function toCanvas(canvas: HTMLCanvasElement, clientX: number, clientY: number): Pt {
  const r = canvas.getBoundingClientRect();
  return { x: clientX - r.left, y: clientY - r.top };
}

export function useCanvasInteraction(args: CanvasInteractionArgs): CanvasInteraction {
  const { canvasRef, rendererRef, scene, selected, setSelected, artboard, commit } = args;
  const [view, setViewState] = useState<View>(IDENTITY_VIEW);
  const [size, setSize] = useState<Size>({ w: 0, h: 0 });
  // ponytail: the readout re-renders App on every pointer move. Fine at today's App size; if a
  // profile ever shows it, feed a small readout component from a ref instead of state.
  const [cursor, setCursor] = useState<Pt | null>(null);
  const [fitEpoch, setFitEpoch] = useState(0);
  // Native listeners are registered once and pointer handlers run between renders; both read
  // these rather than a render's closure, so they see the selection the user just made.
  const viewRef = useRef(view);
  const latest = useRef({ scene, selected });
  latest.current = { scene, selected };
  const gesture = useRef<Gesture | null>(null);
  const spaceHeld = useRef(false);

  const setView = useCallback((v: View) => {
    viewRef.current = v;
    setViewState(v);
  }, []);
  const requestFit = useCallback(() => setFitEpoch((n) => n + 1), []);
  const minScale = useMemo(
    () => (artboard && size.w > 0 && size.h > 0 ? minScaleFor(artboard, size) : 0),
    [artboard, size],
  );
  const zoomBy = useCallback(
    (screen: Pt, factor: number) => setView(zoomAt(viewRef.current, screen, factor, minScale)),
    [minScale, setView],
  );
  const centre = useCallback((): Pt => ({ x: size.w / 2, y: size.h / 2 }), [size]);

  // Puts the renderer back on the committed scene: after a click that moved nothing, a refused
  // commit, or a cancelled pointer. Without it the optimistic preview is stranded on screen.
  const restore = useCallback(() => {
    const r = rendererRef.current;
    if (!r) return;
    const { scene: s, selected: sel } = latest.current;
    r.setScene(s);
    r.setSelection(sel);
    r.setOverlay({ box: selectionBox(s, sel), marquee: null });
    r.draw();
  }, [rendererRef]);

  // ponytail: devicePixelRatio is read on resize only, so dragging the window from a Retina panel
  // to a 1x one keeps the old backing store until the next resize. Upgrade: a
  // matchMedia(`(resolution: ${dpr}dppx)`) listener that re-runs resize().
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      rendererRef.current?.resize(width, height, window.devicePixelRatio || 1);
      setSize({ w: width, h: height });
    });
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [canvasRef, rendererRef]);

  // Fit on first layout, on a new artboard (machine switch, project open) and on request. The
  // artboard is compared by its key, not the object: a snapshot rebuilds the object on every
  // edit, and an edit is not a reason to throw away the operator's zoom.
  const artboardKey = artboard ? `${artboard.x},${artboard.y},${artboard.w},${artboard.h}` : null;
  const fittedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!artboard || size.w === 0 || size.h === 0) return;
    const key = `${artboardKey}#${fitEpoch}`;
    if (fittedFor.current === key) return;
    fittedFor.current = key;
    setView(fitView(artboard, size));
  }, [artboardKey, fitEpoch, size, setView]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (e: WheelEvent) => {
      // Registered natively because React's wheel listener is passive; without preventDefault the
      // webview scrolls or page-zooms instead of the canvas.
      e.preventDefault();
      const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? LINE_PX : 1;
      // WebKit reports a trackpad pinch as a wheel with ctrlKey set, so one branch serves both.
      if (e.ctrlKey || e.metaKey) zoomBy(toCanvas(canvas, e.clientX, e.clientY), wheelFactor(e.deltaY * unit));
      else setView(panBy(viewRef.current, -e.deltaX * unit, -e.deltaY * unit));
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [canvasRef, setView, zoomBy]);

  useEffect(() => {
    // Space on a focused button presses it, and ⌘= in a field belongs to the field.
    const ignores = (t: EventTarget | null) =>
      t instanceof HTMLElement && (["INPUT", "TEXTAREA", "SELECT", "BUTTON"].includes(t.tagName) || t.isContentEditable);
    const onDown = (e: KeyboardEvent) => {
      if (ignores(e.target)) return;
      if (e.code === "Space") {
        spaceHeld.current = true;
        e.preventDefault();
        return;
      }
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "0") requestFit();
      else if (e.key === "1") zoomBy(centre(), CSS_PX_PER_MM / viewRef.current.scale);
      else if (e.key === "=" || e.key === "+") zoomBy(centre(), ZOOM_STEP);
      else if (e.key === "-") zoomBy(centre(), 1 / ZOOM_STEP);
      else return;
      // The webview has its own page zoom on these chords; the canvas zoom replaces it.
      e.preventDefault();
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.code === "Space") spaceHeld.current = false;
    };
    // A Space released while another window had focus never arrives here.
    const onBlur = () => {
      spaceHeld.current = false;
    };
    window.addEventListener("keydown", onDown);
    window.addEventListener("keyup", onUp);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onDown);
      window.removeEventListener("keyup", onUp);
      window.removeEventListener("blur", onBlur);
    };
  }, [centre, requestFit, zoomBy]);

  const onPointerDown = (e: PointerEvent<HTMLCanvasElement>) => {
    const screen = toCanvas(e.currentTarget, e.clientX, e.clientY);
    const v = viewRef.current;
    const p = screenToWorld(v, screen);
    if (e.button === 1 || (e.button === 0 && spaceHeld.current)) {
      e.currentTarget.setPointerCapture(e.pointerId);
      gesture.current = { t: "pan", last: screen };
      return;
    }
    if (e.button !== 0) return;
    // Capture replaces SP3's window-level mouseup listener: a drag released outside the canvas
    // still ends here, so its preview is committed instead of stranded.
    e.currentTarget.setPointerCapture(e.pointerId);
    const { scene: s, selected: sel } = latest.current;
    const box = selectionBox(s, sel);
    const kind = box ? handleAt(box, p, HANDLE_HIT_PX / v.scale, ROTATE_ZONE_PX / v.scale) : null;
    // Shift-click inside the box toggles the node under the pointer rather than dragging.
    if (box && kind && !(kind === "move" && e.shiftKey)) {
      gesture.current = { t: "transform", kind, box, ids: sel, start: p, m: IDENTITY };
      return;
    }
    const hit = hitTest(s, p.x, p.y, HIT_TOL_PX / v.scale);
    if (hit === null) {
      gesture.current = { t: "marquee", start: p, cur: p, additive: e.shiftKey };
      return;
    }
    const next = e.shiftKey ? toggleId(sel, hit) : [hit];
    setSelected(next);
    // Only drag when the hit node is in the new selection — a Shift-click that toggles a node
    // out must not start dragging the rest.
    const nextBox = next.includes(hit) ? selectionBox(s, next) : null;
    gesture.current = nextBox ? { t: "transform", kind: "move", box: nextBox, ids: next, start: p, m: IDENTITY } : null;
  };

  const onPointerMove = (e: PointerEvent<HTMLCanvasElement>) => {
    const canvas = e.currentTarget;
    const screen = toCanvas(canvas, e.clientX, e.clientY);
    const v = viewRef.current;
    const p = screenToWorld(v, screen);
    setCursor(p);
    const g = gesture.current;
    const r = rendererRef.current;
    if (!g) {
      const box = selectionBox(latest.current.scene, latest.current.selected);
      const kind = spaceHeld.current ? "pan" : box ? handleAt(box, p, HANDLE_HIT_PX / v.scale, ROTATE_ZONE_PX / v.scale) : null;
      // ponytail: resize cursors are screen-aligned, so on a rotated box they point along the
      // screen rather than the edge. Upgrade: choose by the handle's on-screen angle.
      canvas.style.cursor = kind ? CURSORS[kind] : "default";
      return;
    }
    if (g.t === "pan") {
      setView(panBy(v, screen.x - g.last.x, screen.y - g.last.y));
      gesture.current = { ...g, last: screen };
      return;
    }
    if (!r) return;
    if (g.t === "marquee") {
      gesture.current = { ...g, cur: p };
      r.setOverlay({ box: null, marquee: normalizeRect(g.start, p) });
      r.draw();
      return;
    }
    const m = gestureMatrix(g.kind, g.box, g.start, p, { shift: e.shiftKey, alt: e.altKey });
    gesture.current = { ...g, m };
    r.setScene(applyOptimistic(latest.current.scene, g.ids, m));
    r.setSelection(g.ids);
    r.setOverlay({ box: { ...g.box, frame: compose(g.box.frame, m) }, marquee: null });
    r.draw();
  };

  const onPointerUp = () => {
    const g = gesture.current;
    gesture.current = null;
    if (!g || g.t === "pan") return;
    if (g.t === "marquee") {
      const travel = Math.hypot(g.cur.x - g.start.x, g.cur.y - g.start.y) * viewRef.current.scale;
      const { scene: s, selected: sel } = latest.current;
      if (travel < CLICK_SLOP_PX) {
        if (!g.additive) setSelected([]);
      } else {
        setSelected(marqueeSelection(sel, marqueeHits(s, normalizeRect(g.start, g.cur)), g.additive));
      }
      // The band is drawn imperatively, and a selection that did not change re-renders nothing.
      restore();
      return;
    }
    if (isIdentity(g.m)) {
      restore();
      return;
    }
    void commit(g.ids, g.m).then((ok) => {
      if (!ok) restore();
    });
  };

  const onPointerCancel = () => {
    gesture.current = null;
    restore();
  };

  const onPointerLeave = () => setCursor(null);

  return {
    view,
    size,
    cursor,
    requestFit,
    handlers: { onPointerDown, onPointerMove, onPointerUp, onPointerCancel, onPointerLeave },
  };
}
```

- [ ] **Step 2: Wire `App.tsx`**

Delete: `dragStart`, `dragIds` (`App.tsx:146-150`), `canvasPos`, `onCanvasMouseDown`, `onCanvasMouseMove`, `finishDrag`, `onCanvasMouseUp` and the window `mouseup` effect (`App.tsx:296-349`). Drop the now-unused imports (`MouseEvent`, `hitTest`, `applyOptimistic`, `dragMatrix`, `Pt`). Add:

```ts
import { useCanvasInteraction } from "./interaction/useCanvasInteraction";
import { viewMatrix, zoomPercent } from "./interaction/viewport";
import { selectionBox } from "./interaction/selectionBox";
```

Directly after the effect that constructs the renderer (`App.tsx:256-259`), so the renderer exists before the hook's observer first fires:

```ts
  const interaction = useCanvasInteraction({
    canvasRef,
    rendererRef,
    scene,
    selected,
    setSelected,
    artboard: doc?.artboard ?? null,
    commit: (ids, m) => run(() => ipc.commitTransform({ ids, m })),
  });
```

Replace the draw effect (`App.tsx:261-268`):

```ts
  useEffect(() => {
    const r = rendererRef.current;
    if (!r) return;
    r.setScene(scene);
    r.setSelection(selected);
    r.setArtboard(doc?.artboard ?? null);
    r.setView(viewMatrix(interaction.view));
    r.setOverlay({ box: selectionBox(scene, selected), marquee: null });
    r.draw();
    // `size` because a resize clears the backing store, and nothing else would repaint it.
  }, [scene, selected, doc, interaction.view, interaction.size]);
```

In `onOpen` and `onReload`, call `interaction.requestFit();` right after the `loadProject` await, next to `setSelected([])`.

Replace the `<canvas …/>` element with:

```tsx
      {/* `data-view` lets e2e turn document mm into page px; the canvas's pixels are unreadable there. */}
      <div style={{ position: "relative", minWidth: 0, minHeight: 0, overflow: "hidden" }}>
        <canvas
          ref={canvasRef}
          data-testid="design-canvas"
          data-view={`${interaction.view.scale} ${interaction.view.tx} ${interaction.view.ty}`}
          style={{ position: "absolute", inset: 0, width: "100%", height: "100%", display: "block", background: "var(--workspace)", touchAction: "none" }}
          {...interaction.handlers}
        />
      </div>
```

The wrapper is why the canvas can fill its grid cell: a canvas sized `100%` directly in the grid contributes its intrinsic 300×150 to the track's minimum and never shrinks.

Pass the readout to the status bar:

```tsx
        <StatusBar
          machine={doc?.machine ?? null}
          artboard={doc?.artboard ?? null}
          error={error}
          status={status}
          zoomPercent={doc ? zoomPercent(interaction.view) : null}
          cursor={interaction.cursor}
        />
```

- [ ] **Step 3: `StatusBar.tsx`** — add to `Props`:

```ts
  /** Null until a document is loaded: there is no artboard to be zoomed relative to. */
  zoomPercent: number | null;
  /** Pointer position in document mm, or null when the pointer is off the canvas. */
  cursor: { x: number; y: number } | null;
```

Destructure them, and replace `<div style={{ flex: 1 }} />` with:

```tsx
      <div style={{ flex: 1 }} />
      {cursor ? (
        <span style={{ fontVariantNumeric: "tabular-nums" }}>
          x {cursor.x.toFixed(1)}  y {cursor.y.toFixed(1)} mm
        </span>
      ) : null}
      {zoomPercent !== null ? (
        <span data-testid="status-zoom" style={{ fontVariantNumeric: "tabular-nums" }}>
          {zoomPercent}%
        </span>
      ) : null}
```

- [ ] **Step 4: `LayersPanel.tsx`** — on the row `div` next to `data-testid="layer-row"`, add `data-selected={isSelected}`.

- [ ] **Step 5: Run unit tests and build, then try it in the app**

Run: `npm --prefix apps/desktop/ui test && npm --prefix apps/desktop/ui run build`
Expected: PASS; `tsc` clean.

Then `(cd apps/desktop && cargo tauri dev)` and check by hand: the artboard fits the window; ⌘-scroll or pinch zooms about the pointer; scrolling pans; Space-drag pans; ⌘0 refits; a selected rect shows eight handles; dragging a corner scales it, outside a corner rotates it; one ⌘Z undoes each gesture.

- [ ] **Step 6: Run the existing e2e suite — expect PASS before Task 10 adds to it**

Run: `npm --prefix apps/desktop/ui run e2e`
Expected: PASS. `new doc → add rect → save → reload` clicks the canvas at (400, 300); that now starts and ends a zero-travel marquee, which clears the selection exactly as the old click did.

- [ ] **Step 7: Commit**

```bash
git add apps/desktop/ui/src apps/desktop/ui/dist
git commit -m "Wire zoom, pan, handles and marquee into the canvas, since SP3 promised them and shipped a fixed 1 px-per-mm view"
```

---

### Task 10: The e2e fake composes the matrix, and three tests

**Files:**
- Modify: `apps/desktop/ui/e2e/smoke.spec.ts`

- [ ] **Step 1: Make the fake's `commit_transform` honest** — replace its body (`smoke.spec.ts:115-124`):

```ts
    commit_transform: (a) => {
      // Composed in full, as `transform_nodes` does: handles send scale and rotation, and a fake
      // that kept only the translation passes a frontend whose preview and commit disagree. The
      // seeded nodes sit directly under an identity Layer, so the world-space `m` composes onto
      // the local transform with no parent conversion. Recorded so a test can read the matrix.
      const m = a.m as number[];
      const hooks = window as unknown as { __commitTransforms?: { ids: number[]; m: number[] }[] };
      hooks.__commitTransforms ??= [];
      hooks.__commitTransforms.push({ ids: a.ids as number[], m });
      for (const id of a.ids as number[]) {
        const node = doc.nodes[id];
        if (!node) continue;
        const [a1, b1, c1, d1, e1, f1] = node.transform;
        const [a2, b2, c2, d2, e2, f2] = m;
        node.transform = [
          a2 * a1 + c2 * b1, b2 * a1 + d2 * b1,
          a2 * c1 + c2 * d1, b2 * c1 + d2 * d1,
          a2 * e1 + c2 * f1 + e2, b2 * e1 + d2 * f1 + f2,
        ];
      }
      return {};
    },
```

- [ ] **Step 2: Add helpers and tests** at the end of the file (add `type Page` to the `@playwright/test` import):

```ts
// The canvas's pixels are unreadable from a test, so these turn document mm into page px with the
// view the canvas publishes as `data-view="scale tx ty"`.
type CanvasView = { scale: number; tx: number; ty: number };

async function readView(page: Page): Promise<CanvasView> {
  const attr = (await page.getByTestId("design-canvas").getAttribute("data-view")) ?? "";
  const [scale, tx, ty] = attr.split(" ").map(Number);
  return { scale, tx, ty };
}

async function fittedView(page: Page): Promise<CanvasView> {
  await expect(page.getByTestId("design-canvas")).not.toHaveAttribute("data-view", "1 0 0");
  return readView(page);
}

async function toPage(page: Page, v: CanvasView, mm: { x: number; y: number }) {
  const box = await page.getByTestId("design-canvas").boundingBox();
  if (!box) throw new Error("design canvas has no layout box");
  return { x: box.x + mm.x * v.scale + v.tx, y: box.y + mm.y * v.scale + v.ty };
}

/** Ctrl-wheel at a document point; returns the view once the zoom has landed. */
async function zoomInAt(page: Page, mm: { x: number; y: number }, deltaY: number): Promise<CanvasView> {
  const before = await fittedView(page);
  const at = await toPage(page, before, mm);
  await page.mouse.move(at.x, at.y);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, deltaY);
  await page.keyboard.up("Control");
  await expect.poll(async () => (await readView(page)).scale).toBeGreaterThan(before.scale);
  return readView(page);
}

test("Ctrl-wheel zooms about the cursor and the status bar reports it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v0 = await fittedView(page);
  const zoom = page.getByTestId("status-zoom");
  const before = parseInt((await zoom.textContent()) ?? "", 10);
  const at = await toPage(page, v0, { x: 5, y: 5 });

  const v1 = await zoomInAt(page, { x: 5, y: 5 }, -300);

  await expect.poll(async () => parseInt((await zoom.textContent()) ?? "", 10)).toBeGreaterThan(before);
  // About the cursor: the document point under it has not moved on the page.
  const after = await toPage(page, v1, { x: 5, y: 5 });
  expect(after.x).toBeCloseTo(at.x, 0);
  expect(after.y).toBeCloseTo(at.y, 0);
});

test("dragging a corner handle commits one scale about the opposite corner", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click(); // the red 10 × 10 mm rect at the origin
  // Far enough in that the handles are tens of px apart.
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  const se = await toPage(page, v, { x: 10, y: 10 });
  await page.mouse.move(se.x, se.y);
  await page.mouse.down();
  await page.mouse.move(se.x + 10 * v.scale, se.y + 5 * v.scale, { steps: 4 });
  await page.mouse.up();

  const commits = await page.evaluate(
    () => (window as unknown as { __commitTransforms?: { ids: number[]; m: number[] }[] }).__commitTransforms ?? [],
  );
  expect(commits).toHaveLength(1); // one gesture, one commit, one undo entry
  expect(commits[0].ids).toEqual([2]);
  // 10 → 20 mm wide, 10 → 15 mm tall, with the nw corner at the origin held still.
  const [a, b, c, d, e, f] = commits[0].m;
  expect(a).toBeCloseTo(2, 1);
  expect(d).toBeCloseTo(1.5, 1);
  for (const zero of [b, c, e, f]) expect(zero).toBeCloseTo(0, 1);
});

test("a marquee over both shapes selects both", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  // Zoomed in so the band's start point is clear of the shapes' hit tolerance.
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  const from = await toPage(page, v, { x: -5, y: -5 });
  const to = await toPage(page, v, { x: 15, y: 15 });
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 5 });
  await page.mouse.up();

  await expect(page.locator('[data-testid="layer-row"][data-selected="true"]')).toHaveCount(2);
});
```

- [ ] **Step 3: Run — expect PASS**

Run: `npm --prefix apps/desktop/ui run e2e`
Expected: all PASS, including the three new tests and the IPC gate's `afterEach` (no refused calls: `commit_transform`'s argument names are unchanged).

If the zoom test sees no `ctrlKey` on the wheel event, this Playwright version is not applying held modifiers to `mouse.wheel`; dispatch the event instead with `page.getByTestId("design-canvas").dispatchEvent("wheel", { deltaY, ctrlKey: true, clientX: at.x, clientY: at.y })` and keep the assertions.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/ui/e2e/smoke.spec.ts
git commit -m "Compose the fake's transforms in full and test zoom, handles and marquee, since a translate-only fake hides scale bugs"
```

---

### Task 11: Manual checks and the frame-rate fixture

**Files:**
- Create: `apps/desktop/ui/scripts/perf-svg.mjs`
- Modify: `apps/desktop/MANUAL-CHECKLIST.md`

- [ ] **Step 1: Create the generator**

```js
// SPDX-License-Identifier: GPL-3.0-or-later
// Writes an SVG of N small closed curves (default 5000) to stdout, for the viewport's manual
// frame-rate check. Generated rather than committed: the number has to be re-measurable, and a
// megabyte of SVG in history buys nothing a 20-line script does not.
//
//   node apps/desktop/ui/scripts/perf-svg.mjs > /tmp/perf.svg      # then Import it
const n = Number(process.argv[2] ?? 5000);
const cols = Math.ceil(Math.sqrt(n));
const pitch = 4;
const paths = [];
for (let i = 0; i < n; i++) {
  const x = (i % cols) * pitch;
  const y = Math.floor(i / cols) * pitch;
  // Curves rather than squares: parsing and stroking cost is in the curves.
  paths.push(`<path d="M${x} ${y + 1.5} C${x} ${y} ${x + 3} ${y} ${x + 3} ${y + 1.5} S${x} ${y + 3} ${x} ${y + 1.5}Z" stroke="#000" fill="none"/>`);
}
const size = cols * pitch;
process.stdout.write(
  `<svg xmlns="http://www.w3.org/2000/svg" width="${size}mm" height="${size}mm" viewBox="0 0 ${size} ${size}">\n${paths.join("\n")}\n</svg>\n`,
);
```

Run: `node apps/desktop/ui/scripts/perf-svg.mjs 10 | head -3` — expect an `<svg …>` line and two `<path>` lines.

- [ ] **Step 2: Add a section to `MANUAL-CHECKLIST.md`** (match the file's existing heading style):

```markdown
## Viewport and handles (spec 2026-10-06)

- [ ] Launch on a Retina Mac — strokes and handles are crisp, not blurred; the artboard fits the window.
- [ ] Trackpad: pinch zooms about the fingers; two-finger scroll pans; ⌘0 refits; ⌘1 shows a 100 mm rect as ≈100 mm on a ruler held to the screen.
- [ ] Mouse (Windows/Linux): the wheel pans; Ctrl-wheel zooms; middle-drag pans; Space-drag pans.
- [ ] Switch machine Cameo 5 ⇄ Puma IV — the view refits to the new artboard. Resizing the window does not refit.
- [ ] Marquee a word of imported text; Shift-drag a corner — it scales proportionally; Alt-drag scales from the centre; one undo reverts each gesture.
- [ ] Rotate a rect 30° with Shift held (snaps in 15° steps), cut it on the Cameo 5, and measure — the cut matches the screen.
- [ ] `node apps/desktop/ui/scripts/perf-svg.mjs > /tmp/perf.svg`, Import it, pan and zoom with the WebKit inspector's frame timeline open — record the frame rate here with the machine and date (target 60 fps).
```

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/ui/scripts/perf-svg.mjs apps/desktop/MANUAL-CHECKLIST.md
git commit -m "Add the viewport's hardware checks and a frame-rate fixture generator, since smooth and true-to-size can only be judged on a device"
```

---

## Verification

Run from the repository root, in this order:

```sh
cargo test --workspace --locked                 # includes the ipc_inventory check: it must not move
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run build && git diff --exit-code apps/desktop/ui/dist   # dist committed and current
npm --prefix apps/desktop/ui run e2e
(cd apps/desktop && cargo tauri dev)            # the definition of done, by hand
```

Definition of done, from the spec: open a large SVG → it fits the window, sharp on Retina → pinch into a letter and pan at 60 fps → marquee a word, Shift-drag a corner to scale it proportionally, rotate it 15° at a time → one undo reverts each gesture → cut it, and the cut matches the screen.

Then the repo's three-stage review (`CLAUDE.md`, *PR review*): Copilot on the PR, `/pr-review-toolkit:review-pr` over `git diff main...HEAD` with `pr-test-analyzer` named, then Codex adversarially with `--base main`. Brief Codex to attack the gesture maths specifically — every handle × {Shift, Alt, both} on a rotated box, and the zero-width box — rather than to look it over.
