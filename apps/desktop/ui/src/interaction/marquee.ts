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
