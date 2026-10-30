// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { pathBounds, pathRings } from "./pathdata";

describe("pathBounds", () => {
  it("covers M/L/C coordinates", () => {
    expect(pathBounds("M10,20 L30,40 Z")).toEqual({ x: 10, y: 20, w: 20, h: 20 });
  });
  it("includes cubic control points (conservative)", () => {
    const b = pathBounds("M0,0 C0,50 100,50 100,0 Z");
    expect(b).toEqual({ x: 0, y: 0, w: 100, h: 50 });
  });
  it("handles scientific notation and multiple subpaths", () => {
    const b = pathBounds("M1e1,0 L20,0 Z M-5,2.5 L0,10 Z");
    expect(b).toEqual({ x: -5, y: 0, w: 25, h: 10 });
  });
  it("returns null when there are no coordinates", () => {
    expect(pathBounds("")).toBeNull();
    expect(pathBounds("Z")).toBeNull();
  });
});

describe("pathRings", () => {
  it("keeps a closed square's corners and drops the repeated close", () => {
    expect(pathRings("M0,0 L10,0 L10,10 L0,10 Z")).toEqual([{ pts: [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }, { x: 0, y: 10 }], closed: true }]);
  });

  it("flattens a cubic past its chord", () => {
    const rings = pathRings("M0,0 C0,10 10,10 10,0");
    expect(rings?.[0].closed).toBe(false);
    expect(rings?.[0].pts.length).toBeGreaterThan(2);
    expect(rings?.[0].pts[0]).toEqual({ x: 0, y: 0 });
    const pts = rings?.[0].pts ?? [];
    expect(pts[pts.length - 1]).toEqual({ x: 10, y: 0 });
  });

  it("refuses a relative command instead of reading it as absolute", () => {
    expect(pathRings("m0,0 l10,0")).toBeNull();
  });
});
