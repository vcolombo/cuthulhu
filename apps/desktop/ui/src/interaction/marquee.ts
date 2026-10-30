// SPDX-License-Identifier: GPL-3.0-or-later
import type { Bounds, Scene } from "../render/hittest";
import type { Pt } from "../render/affine";
import { polygonHits, rectRing, type RegionMode } from "./region";

export type { RegionMode };

export function normalizeRect(a: Pt, b: Pt): Bounds {
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(b.x - a.x), h: Math.abs(b.y - a.y) };
}

/** Left to right — and a drag that never moves in x — keeps only shapes fully inside the band.
 *  Right to left keeps shapes the band crosses. x is the whole switch: Shift adds to the
 *  selection, and Alt is the lasso's containment key, so the band has no modifier of its own. */
export function marqueeMode(start: Pt, cur: Pt): RegionMode {
  return cur.x >= start.x ? "contain" : "intersect";
}

export function pointInRect(b: Bounds, p: Pt): boolean {
  return p.x >= b.x && p.x <= b.x + b.w && p.y >= b.y && p.y <= b.y + b.h;
}

/** Both corners, so a drag that repositions a band does not change its size or its mode. */
export function translateCorners(start: Pt, cur: Pt, by: Pt): { start: Pt; cur: Pt } {
  return {
    start: { x: start.x + by.x, y: start.y + by.y },
    cur: { x: cur.x + by.x, y: cur.y + by.y },
  };
}

/** Nodes whose geometry meets the band. `mode` defaults to crossing, which is what a caller
 *  that has not picked a direction means — a word of letters is selected by touching it. */
export function marqueeHits(scene: Scene, band: Bounds, mode: RegionMode = "intersect"): number[] {
  return polygonHits(scene, rectRing(band), mode);
}

/** Nodes whose geometry meets a freeform loop. `contain` keeps a shape only when its whole
 *  outline is inside; `intersect` also keeps a closed shape that has the loop in its fill.
 *  Even-odd, so a loop drawn in a hole misses. Fewer than three points is a click, not a loop. */
export function lassoHits(scene: Scene, loop: Pt[], mode: RegionMode): number[] {
  return polygonHits(scene, loop, mode);
}

export function marqueeSelection(prev: number[], hits: number[], additive: boolean): number[] {
  if (!additive) return hits;
  // A Set, because a Shift-band over thousands of already-selected shapes is the fixture's case.
  const had = new Set(prev);
  return [...prev, ...hits.filter((id) => !had.has(id))];
}

export function toggleId(ids: number[], id: number): number[] {
  return ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id];
}

type TreeNode = { kind: unknown; children: number[] };

/** Every shape at or beneath `ids`, each once, in the order first met. A selected Group or Layer
 *  commits as its whole subtree (`transform_nodes` moves a container with everything in it), so
 *  the box and the preview have to cover the same shapes or the unpreviewed ones jump on release
 *  (Copilot on #298). The scene holds shapes only, which is why this walks the document. */
export function shapesUnder(nodes: Record<string, TreeNode>, ids: number[]): number[] {
  const out = new Set<number>();
  const walk = (id: number) => {
    const n = nodes[id];
    if (!n) return;
    if (typeof n.kind === "object" && n.kind !== null && "Shape" in n.kind) out.add(id);
    else n.children.forEach(walk);
  };
  ids.forEach(walk);
  return [...out];
}

/** The selected ids no other selected id contains, in selection order, each once. A node beneath a
 *  selected container moves with it and only with it (`transform_nodes` skips it), so align and
 *  distribute must not count it as a unit of its own. Once, because `transform_each` applies an id
 *  listed twice twice, and a node listed under two parents in a hand-edited manifest is hit twice. */
export function outermost(nodes: Record<string, TreeNode>, ids: number[]): number[] {
  const beneath = new Set<number>();
  const walk = (id: number) => {
    for (const c of nodes[id]?.children ?? []) {
      beneath.add(c);
      walk(c);
    }
  };
  ids.forEach(walk);
  return [...new Set(ids)].filter((id) => !beneath.has(id));
}
