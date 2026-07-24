// SPDX-License-Identifier: GPL-3.0-or-later
import type { Scene } from "../render/hittest";
export type Pt = { x: number; y: number };
export type Matrix = [number, number, number, number, number, number]; // a b c d e f

export function dragMatrix(start: Pt, cur: Pt): Matrix {
  return [1, 0, 0, 1, cur.x - start.x, cur.y - start.y];
}
export function applyOptimistic(scene: Scene, ids: number[], m: Matrix): Scene {
  return { nodes: scene.nodes.map(n =>
    ids.includes(n.id)
      ? {
          ...n,
          bounds: { ...n.bounds, x: n.bounds.x + m[4], y: n.bounds.y + m[5] },
          world: n.world
            ? ([n.world[0], n.world[1], n.world[2], n.world[3],
                n.world[4] + m[4], n.world[5] + m[5]] as typeof n.world)
            : n.world,
        }
      : n) };
}
export type DeltaOp = { op: "add" | "update" | "remove"; nodeId: number; patch?: any };
export function reconcile(scene: Scene, delta: DeltaOp[]): Scene {
  let nodes = scene.nodes.slice();
  for (const d of delta) {
    if (d.op === "update") nodes = nodes.map(n => n.id === d.nodeId ? { ...n, ...d.patch } : n);
    else if (d.op === "remove") nodes = nodes.filter(n => n.id !== d.nodeId);
    else if (d.op === "add") nodes.push(d.patch);
  }
  return { nodes };
}
