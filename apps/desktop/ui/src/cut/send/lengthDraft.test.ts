// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { unitToMm } from "../../length";
import { inchReady, lengthFromDraft, lengthToDraft, weedMessage } from "./StepPasses";

describe("inch weed fields", () => {
  it("shows the millimetre draft in inches, and writes inches back as millimetres", () => {
    expect(lengthFromDraft("3")).toBe("0.12");
    expect(lengthToDraft("1")).toBe("25.4");
  });

  it("leaves an empty field and a non-numeric fragment unconverted", () => {
    expect(lengthFromDraft("")).toBe("");
    expect(lengthToDraft("")).toBe("");
    expect(lengthFromDraft("abc")).toBe("abc");
    expect(lengthToDraft("abc")).toBe("abc");
    expect(lengthToDraft(".")).toBe(".");
  });

  it("keeps a half-typed inch as text, so 1. does not become one inch", () => {
    expect(inchReady("1.")).toBe(false);
    expect(lengthToDraft("1.")).toBe("1.");
    expect(inchReady("1")).toBe(true);
    expect(inchReady("")).toBe(true);
  });

  it("quotes a weed bound in the unit the field is showing", () => {
    expect(weedMessage("Margin must be 0.5–50 mm", "mm")).toBe("Margin must be 0.5–50 mm");
    // Hundredths that round-trip: 1.97 in is 50.038 mm, which the millimetre check refuses.
    expect(weedMessage("Margin must be 0.5–50 mm", "in")).toBe("Margin must be 0.02–1.96 in");
    expect(weedMessage("Spacing must be 5–500 mm", "in")).toBe("Spacing must be 0.20–19.68 in");
    expect(weedMessage("Margin must be 25.4–25.4 mm", "in")).toBe("Margin must be 1.00–1.00 in");
    expect(unitToMm(0.02, "in")).toBeGreaterThanOrEqual(0.5);
    expect(unitToMm(1.96, "in")).toBeLessThanOrEqual(50);
    expect(unitToMm(0.2, "in")).toBeGreaterThanOrEqual(5);
    expect(unitToMm(19.68, "in")).toBeLessThanOrEqual(500);
    expect(weedMessage("Margin must be a number of mm", "in")).toBe("Margin must be a number of in");
    expect(weedMessage("Clearance must be less than the margin", "in")).toBe("Clearance must be less than the margin");
  });
});
