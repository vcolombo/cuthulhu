// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { marqueeHits, marqueeSelection, normalizeRect, outermost, shapesUnder, toggleId } from "./marquee";

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
