// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { cssColor } from "./CutPreview";

describe("cssColor", () => {
  it("unpacks 0xRRGGBBAA into an rgba() string", () => {
    expect(cssColor(0xff0000ff)).toBe("rgba(255, 0, 0, 1)");
    expect(cssColor(0x00ff0080)).toBe("rgba(0, 255, 0, 0.5019607843137255)");
  });
  it("handles zero alpha", () => {
    expect(cssColor(0x00000000)).toBe("rgba(0, 0, 0, 0)");
  });
});
