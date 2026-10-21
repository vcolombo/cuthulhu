// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, test } from "vitest";
import { formatLength, unitForLocale, unitFromPreference, unitToMm } from "./length";

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

describe("unit for a locale", () => {
  test("US customary regions are inches", () => {
    for (const locale of ["en-US", "en-us", "en_US", "en-US-u-ca-gregory", "zh-Hans-US", "en-LR", "my-MM"]) {
      expect(unitForLocale(locale), locale).toBe("in");
    }
  });

  test("everyone else is millimetres, including a locale with no region", () => {
    for (const locale of ["en-GB", "en", "fr-FR", "es-MX", "de", "es-419", "", "not a locale"]) {
      expect(unitForLocale(locale), locale).toBe("mm");
    }
  });

  test("a saved unit is kept, and an absent one is resolved once", () => {
    expect(unitFromPreference("mm", "en-US")).toEqual({ unit: "mm", save: false });
    expect(unitFromPreference("in", "fr-FR")).toEqual({ unit: "in", save: false });
    expect(unitFromPreference(null, "en-US")).toEqual({ unit: "in", save: true });
    expect(unitFromPreference(undefined, "fr-FR")).toEqual({ unit: "mm", save: true });
  });
});
