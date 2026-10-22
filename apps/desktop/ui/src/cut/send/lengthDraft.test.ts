// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
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
    expect(weedMessage("Margin must be 0.5–50 mm", "in")).toBe("Margin must be 0.02–1.97 in");
    expect(weedMessage("Margin must be a number of mm", "in")).toBe("Margin must be a number of in");
    expect(weedMessage("Clearance must be less than the margin", "in")).toBe("Clearance must be less than the margin");
  });
});
