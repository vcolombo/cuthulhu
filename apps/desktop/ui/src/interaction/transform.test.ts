// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { applyMoves, applyOptimistic, reconcile, gestureScene } from "./transform";
import { rotateAbout } from "../render/affine";
import type { Affine6 } from "../render/hittest";

describe("optimistic transform", () => {
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
  it("applyOptimistic nudges the world transform when present", () => {
    const scene = {
      nodes: [{ id: 1, bounds: { x: 0, y: 0, w: 4, h: 4 },
                shape: { t: "rect" as const, w: 4, h: 4 },
                world: [1, 0, 0, 1, 0, 0] as [number, number, number, number, number, number] }],
    };
    const out = applyOptimistic(scene, [1], [1, 0, 0, 1, 7, 3]);
    expect(out.nodes[0].world).toEqual([1, 0, 0, 1, 7, 3]);
    expect(out.nodes[0].bounds.x).toBe(7);
  });
  it("applyOptimistic previews a rotation through world and bounds", () => {
    const scene = {
      nodes: [{ id: 1, bounds: { x: 0, y: 0, w: 10, h: 10 }, local: { x: 0, y: 0, w: 10, h: 10 },
                world: [1, 0, 0, 1, 0, 0] as Affine6 }],
    };
    const out = applyOptimistic(scene, [1], rotateAbout(Math.PI / 4, { x: 5, y: 5 }));
    expect(out.nodes[0].bounds.w).toBeCloseTo(10 * Math.SQRT2, 9);
    expect(out.nodes[0].world![1]).toBeCloseTo(Math.SQRT1_2, 9);
  });
});

describe("gestureScene", () => {
  const committed = { nodes: [{ id: 1, bounds: { x: 0, y: 0, w: 4, h: 4 } }] };
  const preview = applyOptimistic(committed, [1], [1, 0, 0, 1, 10, 0]);

  it("keeps the preview through any snapshot while its commit is on the wire", () => {
    // An earlier commit's snapshot can render mid-flight; it does not include this commit.
    expect(gestureScene({ preview, retireAt: Infinity }, committed, 7)).toBe(preview);
  });

  it("retires the preview once the snapshot that includes its commit has rendered", () => {
    const refreshed = { nodes: [{ id: 1, bounds: { x: 10, y: 0, w: 4, h: 4 } }] };
    expect(gestureScene({ preview, retireAt: 4 }, committed, 3)).toBe(preview);
    expect(gestureScene({ preview, retireAt: 4 }, refreshed, 4)).toBe(refreshed);
  });

  it("uses the committed scene when nothing is pending", () => {
    expect(gestureScene(null, committed, 0)).toBe(committed);
  });
});

describe("applyMoves", () => {
  const box = (id: number) => ({ id, bounds: { x: 0, y: 0, w: 4, h: 4 } });

  it("moves each entry's shapes by its own matrix in one pass, leaving the rest", () => {
    const scene = { nodes: [box(1), box(2), box(3)] };
    const out = applyMoves(scene, [
      { shapes: [1], m: [1, 0, 0, 1, 5, 0] },
      { shapes: [3], m: [1, 0, 0, 1, 0, 7] },
    ]);
    expect(out.nodes.map((n) => [n.bounds.x, n.bounds.y])).toEqual([[5, 0], [0, 0], [0, 7]]);
  });

  it("matches applying the moves one at a time when they share no shape", () => {
    // Align's units are disjoint (`outermost`), which is what lets one pass stand in for many.
    const scene = { nodes: Array.from({ length: 50 }, (_, i) => box(i)) };
    const moves = Array.from({ length: 25 }, (_, i) => ({ shapes: [2 * i], m: [1, 0, 0, 1, i, -i] as Affine6 }));
    const oneByOne = moves.reduce((s, mv) => applyOptimistic(s, mv.shapes, mv.m), scene);
    expect(applyMoves(scene, moves)).toEqual(oneByOne);
  });
});

describe("applyMoves with a shape listed twice", () => {
  it("composes the entries in order, as transform_each moves an id listed twice by both", () => {
    // Type-design review on #301: the preview kept only the last matrix, so it disagreed with the
    // backend until the snapshot replaced it. Translate then scale about the origin: 1 → 6 → 12.
    const scene = { nodes: [{ id: 1, bounds: { x: 1, y: 0, w: 2, h: 2 } }] };
    const moves = [
      { shapes: [1], m: [1, 0, 0, 1, 5, 0] as Affine6 },
      { shapes: [1], m: [2, 0, 0, 2, 0, 0] as Affine6 },
    ];
    const oneByOne = moves.reduce((s, mv) => applyOptimistic(s, mv.shapes, mv.m), scene);
    expect(applyMoves(scene, moves)).toEqual(oneByOne);
    expect(applyMoves(scene, moves).nodes[0].bounds.x).toBe(12);
  });
});
