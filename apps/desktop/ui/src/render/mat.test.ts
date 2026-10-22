// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { MM_PER_IN } from "../length";
import {
  ROLL_DEFAULT_MM,
  ROLL_MIN_MM,
  clampRollLength,
  defaultMatKind,
  matBounds,
  matKindLabel,
  matKindsFor,
  offeredKind,
} from "./mat";

describe("mat presets", () => {
  it("offers Cameo three mats and a Puma a roll, including when no machine is chosen", () => {
    expect(matKindsFor("cameo5")).toEqual(["12x12", "12x24", "roll"]);
    expect(matKindsFor(null)).toEqual(["12x12", "12x24", "roll"]);
    expect(matKindsFor("puma")).toEqual(["roll"]);
    expect(defaultMatKind("puma")).toBe("roll");
    expect(defaultMatKind(null)).toBe("12x12");
  });

  it("keeps a 12×24 in hand while a Puma is showing its roll", () => {
    expect(offeredKind("12x24", "puma")).toBe("roll");
    expect(offeredKind("12x24", "cameo5")).toBe("12x24");
  });

  it("clamps a sheet to the machine width and a roll to the machine's full width", () => {
    expect(matBounds("12x12", 330, ROLL_DEFAULT_MM)).toEqual({ x: 0, y: 0, w: 12 * MM_PER_IN, h: 12 * MM_PER_IN });
    expect(matBounds("12x24", 330, ROLL_DEFAULT_MM)).toEqual({ x: 0, y: 0, w: 12 * MM_PER_IN, h: 24 * MM_PER_IN });
    expect(matBounds("12x12", 200, ROLL_DEFAULT_MM).w).toBe(200);
    expect(matBounds("12x12", 200, ROLL_DEFAULT_MM).h).toBe(12 * MM_PER_IN);
    expect(matBounds("roll", 600, ROLL_DEFAULT_MM)).toEqual({ x: 0, y: 0, w: 600, h: 36 * MM_PER_IN });
    expect(matBounds("roll", 600, 10).h).toBe(ROLL_MIN_MM);
    expect(clampRollLength(Number.NaN)).toBe(ROLL_DEFAULT_MM);
  });

  it("names sheets in the unit the status bar is using", () => {
    expect(matKindLabel("12x12", "in")).toBe("12 × 12 in");
    expect(matKindLabel("12x24", "in")).toBe("12 × 24 in");
    expect(matKindLabel("12x12", "mm")).toBe("304.8 × 304.8 mm");
    expect(matKindLabel("roll", "mm")).toBe("Roll");
  });
});
