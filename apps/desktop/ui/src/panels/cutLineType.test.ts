// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import type { DocNode } from "../App";
import { selectionCutLineType } from "./cutLineType";

// Annotated `DocNode` rather than inferred: `tsconfig.json` compiles `src/**/*.test.ts` under
// `strict`, and an inferred `as const` transform is a *readonly* tuple, which `Affine6`
// (`render/hittest.ts`) is not — the assignment fails to typecheck and takes `npm run build`
// with it.
const shape = (id: number, cut: "Cut" | "NoCut"): DocNode => ({
  id,
  kind: { Shape: { Rect: { w: 1, h: 1 } } },
  transform: [1, 0, 0, 1, 0, 0],
  children: [],
  cut_line_type: cut,
});
const group = (id: number, children: number[]): DocNode => ({
  id,
  kind: "Group",
  transform: [1, 0, 0, 1, 0, 0],
  children,
  cut_line_type: "Cut",
});

describe("selectionCutLineType", () => {
  it("is null with nothing selected, so the panel can hide the control", () => {
    expect(selectionCutLineType({ 1: shape(1, "Cut") }, [])).toBeNull();
  });

  it("reads the shapes under a selected container, since the attribute does not inherit", () => {
    const nodes = { 1: group(1, [2]), 2: shape(2, "NoCut") };
    expect(selectionCutLineType(nodes, [1])).toBe("NoCut");
  });

  it("is mixed when the selection disagrees, so neither value is shown as the truth", () => {
    const nodes = { 1: shape(1, "Cut"), 2: shape(2, "NoCut") };
    expect(selectionCutLineType(nodes, [1, 2])).toBe("mixed");
  });

  it("is null for a selection with no shapes under it at all", () => {
    expect(selectionCutLineType({ 1: group(1, []) }, [1])).toBeNull();
  });
});
