// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { hitTest } from "./hittest";

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
});
