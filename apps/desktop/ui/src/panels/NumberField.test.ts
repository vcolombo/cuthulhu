// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { scrubValue } from "./NumberField";

describe("scrubValue", () => {
  it("changes value by dx * step", () => {
    expect(scrubValue(10, 4, 0.5)).toBe(12); // 10 + 4*0.5
  });
  it("respects a min clamp", () => {
    expect(scrubValue(1, -100, 1, 0)).toBe(0);
  });
});
