// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { MM_PER_IN } from "../length";
import { gridStep } from "./Canvas2DRenderer";

describe("gridStep", () => {
  const minor = MM_PER_IN / 4;
  const major = MM_PER_IN;

  it("keeps the quarter-inch grid on a fitted 12 in mat", () => {
    expect(gridStep(minor, major, 2, 12 * MM_PER_IN)).toBe(minor);
  });

  it("drops to the major step when minor lines would be closer than 4 px", () => {
    expect(gridStep(5, 10, 0.5, 100)).toBe(10);
  });

  it("coarsens by whole major steps when a long roll would pass the line cap", () => {
    const span = 50_000;
    const step = gridStep(minor, major, 0.01, span);
    expect(step).toBeGreaterThan(major);
    expect(step / major).toBeCloseTo(Math.round(step / major), 6);
    expect(span / step).toBeLessThanOrEqual(499);
  });
});
