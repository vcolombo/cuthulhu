// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, test } from "vitest";
import { formatLength, unitToMm } from "./length";

describe("length units", () => {
  test("1.000 in stores 25.4 mm and shows 25.4 mm", () => {
    const stored = unitToMm(1, "in");
    expect(stored).toBe(25.4);
    expect(formatLength(stored, "mm")).toBe("25.4");
    expect(formatLength(stored, "in")).toBe("1.00");
  });

  test("millimetres round-trip without a conversion", () => {
    expect(unitToMm(12.5, "mm")).toBe(12.5);
    expect(formatLength(12.5, "mm")).toBe("12.5");
  });
});
