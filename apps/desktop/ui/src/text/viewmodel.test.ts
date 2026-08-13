// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { canInsert, fontsLoaded, selectFamily } from "./viewmodel";

describe("fontsLoaded", () => {
  it("maps an empty list to the empty state, not a blank selection", () => {
    expect(fontsLoaded([])).toEqual({ kind: "empty" });
  });

  it("selects the first family when the list is non-empty", () => {
    expect(fontsLoaded(["A", "B"])).toEqual({ kind: "ready", families: ["A", "B"], selected: "A" });
  });
});

describe("selectFamily", () => {
  it("switches selection within listed families", () => {
    const ready = fontsLoaded(["A", "B"]);
    expect(selectFamily(ready, "B")).toEqual({ kind: "ready", families: ["A", "B"], selected: "B" });
  });

  it("ignores a family the backend never listed", () => {
    const ready = fontsLoaded(["A", "B"]);
    expect(selectFamily(ready, "C")).toBe(ready);
  });

  it("ignores selection in non-ready states", () => {
    const loading = { kind: "loading" } as const;
    expect(selectFamily(loading, "A")).toBe(loading);
    const empty = fontsLoaded([]);
    expect(selectFamily(empty, "A")).toBe(empty);
  });
});

describe("canInsert", () => {
  it("permits insert only when a listed family is selected", () => {
    expect(canInsert(fontsLoaded(["A"]))).toBe(true);
    expect(canInsert(fontsLoaded([]))).toBe(false);
    expect(canInsert({ kind: "loading" })).toBe(false);
    expect(canInsert({ kind: "error", message: "boom" })).toBe(false);
  });
});
