// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, test } from "vitest";
import { resolveAppearance } from "./theme";

describe("resolveAppearance", () => {
  test("an explicit choice ignores the operating system", () => {
    expect(resolveAppearance("light", "dark")).toBe("light");
    expect(resolveAppearance("dark", "light")).toBe("dark");
  });

  test("system is whichever theme the operating system is", () => {
    expect(resolveAppearance("system", "dark")).toBe("dark");
    expect(resolveAppearance("system", "light")).toBe("light");
  });
});
