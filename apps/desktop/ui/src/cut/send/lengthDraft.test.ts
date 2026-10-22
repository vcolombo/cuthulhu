// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { lengthFromDraft, lengthToDraft } from "./StepPasses";

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
});
