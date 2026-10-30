// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { lassoHits, marqueeHits, marqueeMode, marqueeSelection, normalizeRect, outermost, shapesUnder, toggleId, translateCorners } from "./marquee";
import { IDENTITY, rotateAbout } from "../render/affine";

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

  it("drags left-to-right contain and right-to-left cross", () => {
    expect(marqueeMode({ x: 0, y: 0 }, { x: 10, y: 4 })).toBe("contain");
    expect(marqueeMode({ x: 0, y: 0 }, { x: 0, y: 4 })).toBe("contain");
    expect(marqueeMode({ x: 10, y: 0 }, { x: 0, y: 4 })).toBe("intersect");
  });

  it("moves both corners, so a repositioned band keeps its size", () => {
    expect(translateCorners({ x: 1, y: 2 }, { x: 5, y: 0 }, { x: 3, y: -1 })).toEqual({
      start: { x: 4, y: 1 }, cur: { x: 8, y: -1 },
    });
  });
});

describe("geometry selection", () => {
  const ell = {
    id: 1,
    bounds: { x: 0, y: 0, w: 20, h: 20 },
    local: { x: 0, y: 0, w: 20, h: 20 },
    world: IDENTITY,
    shape: { t: "ellipse" as const, rx: 10, ry: 10 },
  };
  const ellL = "M0,0 L10,0 L10,2 L2,2 L2,10 L0,10 Z";
  const ellPath = {
    id: 2,
    bounds: { x: 0, y: 0, w: 10, h: 10 },
    world: IDENTITY,
    shape: { t: "path" as const, d: ellL },
  };

  it("does not select an ellipse by the empty corner of its box", () => {
    // The corner of the 20 mm box is outside the circle. A bounds test would take it.
    expect(marqueeHits({ nodes: [ell] }, { x: -1, y: -1, w: 2, h: 2 })).toEqual([]);
    expect(marqueeHits({ nodes: [ell] }, { x: 9, y: 9, w: 2, h: 2 })).toEqual([1]);
  });

  it("selects a closed shape when the band sits in its fill and touches no edge", () => {
    const rect = {
      id: 3,
      bounds: { x: 0, y: 0, w: 30, h: 30 },
      world: IDENTITY,
      shape: { t: "rect" as const, w: 30, h: 30 },
    };
    expect(marqueeHits({ nodes: [rect] }, { x: 10, y: 10, w: 4, h: 4 })).toEqual([3]);
    expect(marqueeHits({ nodes: [rect] }, { x: 10, y: 10, w: 4, h: 4 }, "contain")).toEqual([]);
    // Snug on the outline, sharing every edge. Touching the boundary is still inside.
    expect(marqueeHits({ nodes: [rect] }, { x: 0, y: 0, w: 30, h: 30 }, "contain")).toEqual([3]);
  });

  it("misses the empty notch of an L and still hits the ink", () => {
    expect(marqueeHits({ nodes: [ellPath] }, { x: 6, y: 6, w: 3, h: 3 })).toEqual([]);
    expect(marqueeHits({ nodes: [ellPath] }, { x: 0, y: 0, w: 3, h: 1 })).toEqual([2]);
  });

  it("follows a rotated world matrix, which is where a nested Group leaves the shape", () => {
    const world = rotateAbout(Math.PI / 2, { x: 0, y: 0 });
    const turned = {
      id: 4,
      // 10×10 rect at the origin, turned 90° about the origin: x −10..0, y 0..10.
      bounds: { x: -10, y: 0, w: 10, h: 10 },
      world,
      shape: { t: "rect" as const, w: 10, h: 10 },
    };
    expect(marqueeHits({ nodes: [turned] }, { x: 1, y: 1, w: 2, h: 2 })).toEqual([]);
    expect(marqueeHits({ nodes: [turned] }, { x: -4, y: 2, w: 2, h: 2 })).toEqual([4]);
  });

  it("misses a hole and does not fill an open path", () => {
    const holed = {
      id: 7,
      bounds: { x: 0, y: 0, w: 30, h: 30 },
      world: IDENTITY,
      shape: { t: "path" as const, d: "M0,0 L30,0 L30,30 L0,30 Z M10,10 L20,10 L20,20 L10,20 Z" },
    };
    expect(marqueeHits({ nodes: [holed] }, { x: 12, y: 12, w: 4, h: 4 })).toEqual([]);
    expect(marqueeHits({ nodes: [holed] }, { x: 0, y: 0, w: 4, h: 4 })).toEqual([7]);
    const open = {
      id: 8,
      bounds: { x: 0, y: 0, w: 10, h: 10 },
      world: IDENTITY,
      shape: { t: "path" as const, d: "M0,0 L10,0 L10,10" },
    };
    // The triangle those two edges would close. An open path has no fill there.
    expect(marqueeHits({ nodes: [open] }, { x: 1, y: 1, w: 2, h: 2 })).toEqual([]);
  });

  it("rejects containment when an edge leaves a concave loop between inside ends", () => {
    const bridge = {
      id: 9,
      bounds: { x: 0, y: 0, w: 10, h: 10 },
      world: IDENTITY,
      // Both ends sit in the C's bars. The middle of the segment is in the mouth, so containment
      // has to notice the edge and not only the vertices.
      shape: { t: "path" as const, d: "M8,1 L8,9" },
    };
    const c = [
      { x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 3 }, { x: 3, y: 3 },
      { x: 3, y: 7 }, { x: 10, y: 7 }, { x: 10, y: 10 }, { x: 0, y: 10 },
    ];
    expect(lassoHits({ nodes: [bridge] }, c, "contain")).toEqual([]);
    expect(lassoHits({ nodes: [bridge] }, c, "intersect")).toEqual([9]);
  });

  it("a lasso contains only a shape that is fully inside, and crosses one that is not", () => {
    const rect = {
      id: 5,
      bounds: { x: 0, y: 0, w: 10, h: 10 },
      world: IDENTITY,
      shape: { t: "rect" as const, w: 10, h: 10 },
    };
    const around = [{ x: -1, y: -1 }, { x: 12, y: -1 }, { x: 12, y: 12 }, { x: -1, y: 12 }];
    const corner = [{ x: -1, y: -1 }, { x: 3, y: -1 }, { x: 3, y: 3 }, { x: -1, y: 3 }];
    expect(lassoHits({ nodes: [rect] }, around, "contain")).toEqual([5]);
    expect(lassoHits({ nodes: [rect] }, corner, "contain")).toEqual([]);
    expect(lassoHits({ nodes: [rect] }, corner, "intersect")).toEqual([5]);
  });
});

describe("shapesUnder", () => {
  // root Layer 1 → Group 2 → { rect 3, Group 4 → ellipse 5 }, plus rect 6 under the Layer.
  const shape = { Shape: { Rect: { w: 1, h: 1 } } };
  const nodes = {
    1: { kind: "Layer", children: [2, 6] },
    2: { kind: "Group", children: [3, 4] },
    3: { kind: shape, children: [] },
    4: { kind: "Group", children: [5] },
    5: { kind: shape, children: [] },
    6: { kind: shape, children: [] },
  };

  it("expands a selected container to every shape beneath it", () => {
    // A container's commit moves its whole subtree, so the box and preview must cover the same.
    expect(shapesUnder(nodes, [2])).toEqual([3, 5]);
  });

  it("keeps a selected shape, and counts a shape selected twice over once", () => {
    expect(shapesUnder(nodes, [6, 2, 3])).toEqual([6, 3, 5]);
  });

  it("drops ids the document no longer has", () => {
    expect(shapesUnder(nodes, [99, 6])).toEqual([6]);
  });
});

describe("outermost", () => {
  const shape = { Shape: { Rect: { w: 1, h: 1 } } };
  const nodes = {
    1: { kind: "Layer", children: [2, 6] },
    2: { kind: "Group", children: [3, 4] },
    3: { kind: shape, children: [] },
    4: { kind: "Group", children: [5] },
    5: { kind: shape, children: [] },
    6: { kind: shape, children: [] },
  };

  it("drops an id beneath another selected id, keeping selection order", () => {
    // The backend moves such a node with its ancestor only, so it is not a unit of its own.
    expect(outermost(nodes, [5, 6, 2, 3])).toEqual([6, 2]);
  });

  it("keeps siblings and unrelated ids", () => {
    expect(outermost(nodes, [3, 4, 6])).toEqual([3, 4, 6]);
  });

  it("lists an id once even when the selection names it twice", () => {
    // transform_each applies an id listed in two entries twice; the preview would show it once.
    expect(outermost(nodes, [6, 3, 6])).toEqual([6, 3]);
  });
});
