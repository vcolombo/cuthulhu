// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { pathBounds } from "./pathdata";

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
