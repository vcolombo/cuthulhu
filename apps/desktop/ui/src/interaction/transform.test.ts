// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { dragMatrix, applyOptimistic, reconcile } from "./transform";

describe("optimistic transform", () => {
  it("dragMatrix builds a translation from start→current", () => {
    expect(dragMatrix({ x: 2, y: 3 }, { x: 5, y: 3 })).toEqual([1, 0, 0, 1, 3, 0]);
  });
  it("applyOptimistic offsets only selected node bounds", () => {
    const scene = { nodes: [
      { id: 1, bounds: { x: 0, y: 0, w: 4, h: 4 } },
      { id: 2, bounds: { x: 0, y: 0, w: 4, h: 4 } },
    ]};
    const out = applyOptimistic(scene, [2], [1, 0, 0, 1, 5, 0]);
    expect(out.nodes[1].bounds.x).toBe(5);
    expect(out.nodes[0].bounds.x).toBe(0);
  });
  it("reconcile applies an update op from the authoritative delta", () => {
    const scene = { nodes: [{ id: 1, bounds: { x: 0, y: 0, w: 4, h: 4 } }] };
    const out = reconcile(scene, [{ op: "update", nodeId: 1, patch: { bounds: { x: 9, y: 0, w: 4, h: 4 } } }]);
    expect(out.nodes[0].bounds.x).toBe(9);
  });
});
