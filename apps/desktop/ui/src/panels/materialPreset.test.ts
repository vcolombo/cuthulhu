// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { effectiveMaterials, selectionAssignment, summariseEffectiveMaterial,
  type EffectiveMaterial } from "./materialPreset";
import { materialLabel } from "./PropertiesPanel";
import type { DocNode } from "../App";

const shape = (id: number, material_preset: DocNode["material_preset"]): DocNode => ({
  id,
  kind: { Shape: { Rect: { w: 1, h: 1 } } },
  transform: [1, 0, 0, 1, 0, 0],
  cut_line_type: "Cut",
  material_preset,
  children: [],
});
const layer = (id: number, material_preset: DocNode["material_preset"], children: number[]): DocNode => ({
  id,
  kind: "Layer",
  transform: [1, 0, 0, 1, 0, 0],
  cut_line_type: "Cut",
  material_preset,
  children,
});

const INHERIT = { state: "inherit" } as const;
const UNASSIGNED = { state: "unassigned" } as const;
const HTV = { state: "preset", id: "cameo5-htv" } as const;

describe("selectionAssignment", () => {
  it("reports the one local assignment a selection agrees on", () => {
    const nodes = { "1": shape(1, HTV), "2": shape(2, HTV) };
    expect(selectionAssignment(nodes, [1, 2])).toEqual(HTV);
  });

  // Mixed on the *local* assignment, even though both resolve to the same id: the control
  // edits local values, and saying otherwise would misreport what a click overwrites.
  it("reports mixed when a Layer's own value differs from its child's", () => {
    const nodes = { "1": layer(1, HTV, [2]), "2": shape(2, INHERIT) };
    expect(selectionAssignment(nodes, [1, 2])).toBe("mixed");
  });

  // No descent, unlike the cuttability helper: a selected Layer speaks for itself, because
  // that is what the command writes.
  it("reads a selected Layer's own value, not its children's", () => {
    const nodes = { "1": layer(1, HTV, [2]), "2": shape(2, UNASSIGNED) };
    expect(selectionAssignment(nodes, [1])).toEqual(HTV);
  });

  it("returns undefined when nothing is selected", () => {
    expect(selectionAssignment({}, [])).toBeUndefined();
  });
});

describe("effectiveMaterials", () => {
  // Mirrors the planner's walk (crates/cutplan/src/passes.rs): nearest assigned ancestor wins,
  // and Unassigned stops the chain rather than deferring up it.
  it("resolves each node the way the planner will", () => {
    const nodes = {
      "1": layer(1, HTV, [2, 3, 4]),
      "2": shape(2, INHERIT),
      "3": shape(3, UNASSIGNED),
      "4": shape(4, { state: "preset", id: "cameo5-vinyl-adhesive" }),
    };
    expect(effectiveMaterials(nodes, 1)).toEqual({
      1: "cameo5-htv",
      2: "cameo5-htv",
      3: null,
      4: "cameo5-vinyl-adhesive",
    });
  });

  it("resolves to nothing when no ancestor assigns one", () => {
    const nodes = { "1": layer(1, INHERIT, [2]), "2": shape(2, INHERIT) };
    expect(effectiveMaterials(nodes, 1)).toEqual({ 1: null, 2: null });
  });

  // A malformed document whose nodes contain each other must not spin the walk — the same
  // guard `plan_passes_with` keeps, for the same reason.
  it("does not loop on a document whose nodes contain each other", () => {
    const nodes = { "1": layer(1, HTV, [2]), "2": layer(2, INHERIT, [1]) };
    expect(effectiveMaterials(nodes, 1)).toEqual({ 1: "cameo5-htv", 2: "cameo5-htv" });
  });
});

describe("summariseEffectiveMaterial", () => {
  // The production reduction, not a copy of it in the test. Greptile's and Copilot's P1: two
  // shapes that both say `Inherit` under different Layers agree on their *local* value, so the
  // panel is not "Mixed" on assignment — and reading the first one's material labelled the pair
  // with it. Reverting to `selected[0]` fails here.
  it("is mixed when the selection resolves to more than one material", () => {
    const resolved = { 2: "cameo5-htv", 4: "cameo5-copy-paper" };
    expect(summariseEffectiveMaterial(resolved, [2, 4])).toEqual({ kind: "mixed" });
    expect(summariseEffectiveMaterial(resolved, [2])).toEqual({ kind: "one", id: "cameo5-htv" });
  });

  it("is one answer when they agree, including agreeing on nothing", () => {
    expect(summariseEffectiveMaterial({ 2: "cameo5-htv", 3: "cameo5-htv" }, [2, 3]))
      .toEqual({ kind: "one", id: "cameo5-htv" });
    expect(summariseEffectiveMaterial({ 2: null, 3: null }, [2, 3])).toEqual({ kind: "one", id: null });
    expect(summariseEffectiveMaterial({}, [])).toEqual({ kind: "one", id: null });
  });

  // Greptile's fourth-push P1, at the consumer that has to keep them apart: a preset whose id is
  // literally `mixed` is one answer, not "these disagree". An in-band sentinel fails here.
  it("keeps a preset called mixed apart from a mixed selection", () => {
    const one = summariseEffectiveMaterial({ 2: "mixed" }, [2]);
    expect(one).toEqual({ kind: "one", id: "mixed" });

    const presets = [{ id: "mixed", name: "Mixed Media", machine_id: "cameo5",
                       settings: { speed: 5, force: 20, repeat_count: 1 }, builtin: false }];
    expect(materialLabel({ state: "inherit" }, one, presets)).toBe("Inherited — Mixed Media");
    expect(materialLabel({ state: "inherit" }, { kind: "mixed" }, presets)).toBe("Inherited — Mixed");
  });

  // And the rest of the label's states, since it is what an operator reads before a bulk edit.
  it("labels each assignment state", () => {
    const presets = [{ id: "cameo5-htv", name: "HTV", machine_id: "cameo5",
                       settings: { speed: 5, force: 20, repeat_count: 1 }, builtin: true }];
    const none: EffectiveMaterial = { kind: "one", id: null };
    expect(materialLabel("mixed", none, presets)).toBe("Mixed");
    expect(materialLabel({ state: "unassigned" }, none, presets)).toBe("No preset");
    expect(materialLabel({ state: "preset", id: "cameo5-htv" }, none, presets)).toBe("HTV");
    expect(materialLabel({ state: "preset", id: "gone" }, none, presets)).toBe("Unresolved (gone)");
    expect(materialLabel({ state: "inherit" }, none, presets)).toBe("Inherited — No preset");
    expect(materialLabel({ state: "inherit" }, { kind: "one", id: "cameo5-htv" }, presets))
      .toBe("Inherited — HTV");
  });
});
