// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { hitTest } from "./hittest";
import { apply, compose, rotateAbout, scaleAbout, transformBounds } from "./affine";

describe("hitTest", () => {
  it("returns the topmost node whose bounds contain the point", () => {
    const scene = {
      nodes: [
        { id: 1, bounds: { x: 0, y: 0, w: 10, h: 10 } },
        { id: 2, bounds: { x: 5, y: 5, w: 10, h: 10 } },
      ],
    };
    expect(hitTest(scene, 7, 7)).toBe(2); // 2 is on top and contains the point
    expect(hitTest(scene, 1, 1)).toBe(1);
    expect(hitTest(scene, 99, 99)).toBe(null);
  });

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

  it("keeps the tolerance a world distance under a sheared transform", () => {
    // A 45° node under a 10×/1× parent is sheared: its local axes are no longer at right angles in
    // world, so dividing by axis lengths under- or over-states the margin. Copilot on #298.
    const world = compose(rotateAbout(Math.PI / 4, { x: 0, y: 0 }), scaleAbout(10, 1, { x: 0, y: 0 }));
    const node = { id: 9, bounds: transformBounds(world, { x: 0, y: 0, w: 10, h: 10 }),
                   local: { x: 0, y: 0, w: 10, h: 10 }, world };
    // 0.9 mm outward from the middle of the local x = 10 edge, measured perpendicular to it in world.
    const [, , c, d] = world;
    const len = Math.hypot(c, d);
    let n = { x: d / len, y: -c / len };
    if (n.x * world[0] + n.y * world[1] < 0) n = { x: -n.x, y: -n.y };
    const mid = apply(world, { x: 10, y: 5 });
    const p = { x: mid.x + 0.9 * n.x, y: mid.y + 0.9 * n.y };
    expect(hitTest({ nodes: [node] }, p.x, p.y, 1)).toBe(9);
    expect(hitTest({ nodes: [node] }, p.x, p.y, 0.8)).toBe(null);
  });

  it("a node scaled to nothing cannot be hit", () => {
    const flat = { id: 8, bounds: { x: 0, y: 0, w: 0, h: 0 }, local: { x: 0, y: 0, w: 10, h: 10 },
                   world: [0, 0, 0, 0, 0, 0] as [number, number, number, number, number, number] };
    expect(hitTest({ nodes: [flat] }, 0, 0, 5)).toBe(null);
  });
});
