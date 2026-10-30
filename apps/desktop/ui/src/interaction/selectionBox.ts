// SPDX-License-Identifier: GPL-3.0-or-later
import { unionBounds, type Affine6, type Bounds, type Scene } from "../render/hittest";
import { apply, compose, invert, translate, type Pt } from "../render/affine";

/** The local rectangle [0, w] × [0, h], placed in world by `frame`. Gestures work in this frame,
 *  which is what lets one scale rule serve a rotated single node and an axis-aligned group alike. */
export type Box = { frame: Affine6; w: number; h: number };
export type ScaleHandle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
export type SkewHandle = "skew-n" | "skew-e" | "skew-s" | "skew-w";
export type HandleKind = ScaleHandle | SkewHandle | "rotate" | "move";

/** Screen-constant targets, in CSS px. Callers divide by the view scale before comparing them
 *  to world mm, so a handle stays the same size on screen at every zoom.
 *
 *  - Scale: an 8px square (`HANDLE_DRAW_PX`). Hit radius 6px, 12px across — a mouse target,
 *    deliberately under a 24px fingertip so a small shape can still be dragged between handles.
 *  - Skew: an 8px diamond centred 22px outside the edge midpoint. Hit radius 12px, 24px across,
 *    the touch target. 22 − 12 = 10px of gap past the scale handle's 6px radius, so the two
 *    do not share a press.
 *  - Rotate: an 18px zone outside a corner (36px across that corner), only where a press would
 *    otherwise miss the box. */
export const HANDLE_DRAW_PX = 8;
export const SCALE_HIT_PX = 6;
export const SKEW_OFFSET_PX = 22;
export const SKEW_HIT_PX = 12;
export const ROTATE_HIT_PX = 18;

/** Each handle's place on the box, as fractions of (w, h). */
export const HANDLE_UNIT: Record<ScaleHandle, Pt> = {
  nw: { x: 0, y: 0 }, n: { x: 0.5, y: 0 }, ne: { x: 1, y: 0 }, e: { x: 1, y: 0.5 },
  se: { x: 1, y: 1 }, s: { x: 0.5, y: 1 }, sw: { x: 0, y: 1 }, w: { x: 0, y: 0.5 },
};
export const SCALE_HANDLES = Object.keys(HANDLE_UNIT) as ScaleHandle[];
export const SKEW_HANDLES = ["skew-n", "skew-e", "skew-s", "skew-w"] as const;

const SKEW_EDGE: Record<SkewHandle, ScaleHandle> = {
  "skew-n": "n", "skew-e": "e", "skew-s": "s", "skew-w": "w",
};
/** Outward in local space. Document y grows downward, so the top edge's outward step is −y. */
const SKEW_NORMAL: Record<SkewHandle, Pt> = {
  "skew-n": { x: 0, y: -1 }, "skew-e": { x: 1, y: 0 }, "skew-s": { x: 0, y: 1 }, "skew-w": { x: -1, y: 0 },
};

export function isSkewHandle(h: HandleKind): h is SkewHandle {
  return h === "skew-n" || h === "skew-e" || h === "skew-s" || h === "skew-w";
}

/** Edge skews that have a length to shear. A straight path has no top edge to drag sideways. */
export function skewHandlesFor(box: Box): SkewHandle[] {
  return SKEW_HANDLES.filter((h) => (h === "skew-n" || h === "skew-s" ? box.h > 0 : box.w > 0));
}

// Float slack on the inside test, so a zero-height box (a straight path) keeps an inside.
const INSIDE_EPS = 1e-9;

/** One node: its oriented box, so a rotated rect keeps handles on its edges. Several: the
 *  axis-aligned union of their world bounds — there is no shared orientation to keep. */
export function selectionBox(scene: Scene, ids: number[]): Box | null {
  // A Set, because this runs on every idle pointer move and a marquee can select thousands.
  const wanted = new Set(ids);
  const nodes = scene.nodes.filter((n) => wanted.has(n.id));
  if (nodes.length === 0) return null;
  const only = nodes.length === 1 ? nodes[0] : null;
  if (only?.local && only.world) {
    return { frame: compose(translate(only.local.x, only.local.y), only.world), w: only.local.w, h: only.local.h };
  }
  const u = unionBounds(nodes.map((n) => n.bounds));
  return { frame: translate(u.x, u.y), w: u.w, h: u.h };
}

/** The axis-aligned world box of the selected shapes, for the status bar.
 *
 *  `unionBounds` walks the boxes. A spread `Math.min` throws `RangeError` once a
 *  selection passes the engine's argument limit, and Cmd-A selects every shape on
 *  the mat — the render that paints the status bar is what would throw, with no
 *  error boundary above it. Membership is a set for the same reason `selectionBox`
 *  uses one: this runs on every render, and `includes` on the id list is quadratic. */
export function selectionBounds(scene: Scene, selected: number[]): Bounds | null {
  if (selected.length === 0) return null;
  const wanted = new Set(selected);
  const boxes: Bounds[] = [];
  for (const node of scene.nodes) {
    if (wanted.has(node.id)) boxes.push(node.bounds);
  }
  if (boxes.length === 0) return null;
  return unionBounds(boxes);
}

export function handleLocal(box: Box, h: ScaleHandle): Pt {
  return { x: HANDLE_UNIT[h].x * box.w, y: HANDLE_UNIT[h].y * box.h };
}

export function handleWorld(box: Box, h: ScaleHandle): Pt {
  return apply(box.frame, handleLocal(box, h));
}

/** The skew diamond, `offset` world mm outside the edge. `offset` is a screen size divided by
 *  the view scale, so the diamond stays put on screen when the operator zooms. */
export function skewPoint(box: Box, h: SkewHandle, offset: number): Pt {
  const local = handleLocal(box, SKEW_EDGE[h]);
  const n = SKEW_NORMAL[h];
  const base = apply(box.frame, local);
  const tip = apply(box.frame, { x: local.x + n.x, y: local.y + n.y });
  const len = Math.hypot(tip.x - base.x, tip.y - base.y) || 1;
  return { x: base.x + ((tip.x - base.x) / len) * offset, y: base.y + ((tip.y - base.y) / len) * offset };
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
export function handleAt(
  box: Box,
  p: Pt,
  handleTol: number,
  rotateTol: number,
  skewTol = handleTol,
  skewOffset = 0,
): HandleKind | null {
  for (const h of SCALE_HANDLES) {
    if (canScale(box, h) && dist(handleWorld(box, h), p) <= handleTol) return h;
  }
  // Outside the edge, past the scale square. A zero offset would sit on the scale handle, which
  // already won above; callers pass a screen offset so the two targets stay apart.
  if (skewOffset > 0) {
    for (const h of skewHandlesFor(box)) {
      if (dist(skewPoint(box, h, skewOffset), p) <= skewTol) return h;
    }
  }
  if (inside(box, p)) return "move";
  if (boxCorners(box).some((c) => dist(c, p) <= rotateTol)) return "rotate";
  return null;
}

/** Whether `h` moves an axis the box has any length along. On a straight path the handles that
 *  only scale its zero dimension sit on top of its body, and claiming the press there advertised a
 *  resize that does nothing while making the path undraggable (Copilot on #298). */
function canScale(box: Box, h: ScaleHandle): boolean {
  const unit = HANDLE_UNIT[h];
  return (unit.x !== 0.5 && box.w > 0) || (unit.y !== 0.5 && box.h > 0);
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
