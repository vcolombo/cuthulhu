// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { distributeTitle } from "./AlignRow";

describe("distributeTitle", () => {
  it("is the bare label when distribute can act, and names the reason when it cannot", () => {
    const label = "Distribute horizontal spacing";
    expect(distributeTitle(label, null)).toBe(label);
    expect(distributeTitle(label, "few")).toBe(`${label}: select three or more pieces`);
    expect(distributeTitle(label, "stacked")).toBe(`${label}: more than one piece spans the selection on this axis`);
    expect(distributeTitle(label, "tight")).toBe(`${label}: the pieces do not fit inside the one around them`);
    expect(distributeTitle(label, "crowded")).toBe(`${label}: the pieces overlap too much to space out evenly`);
    expect(distributeTitle(label, "nested")).toBe(`${label}: a piece inside the one around them spans the rest too`);
  });
});
