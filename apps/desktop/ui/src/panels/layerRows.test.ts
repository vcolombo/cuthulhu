// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, test } from "vitest";
import type { DocNode, DocSnapshot } from "../App";
import type { Grouping } from "../ipc";
import { applySiblingReorder, layerPasses, layerRowIds, shapePassKey } from "./layerRows";

const RED = 0xff0000ff;
const GREEN = 0x00ff00ff;
const BLUE = 0x0000ffff;

function shape(id: number, style: DocNode["style"], extra: Partial<DocNode> = {}): DocNode {
  return {
    kind: { Shape: { Rect: { w: 1, h: 1 } } },
    transform: [1, 0, 0, 1, 0, 0],
    style,
    cut_line_type: "Cut",
    material_preset: { state: "inherit" },
    children: [],
    ...extra,
    id,
  };
}

function container(id: number, kind: "Group" | "Layer", children: number[], extra: Partial<DocNode> = {}): DocNode {
  return {
    kind,
    transform: [1, 0, 0, 1, 0, 0],
    style: { stroke: null, fill: null },
    cut_line_type: "Cut",
    material_preset: { state: "inherit" },
    children,
    ...extra,
    id,
  };
}

function doc(nodes: DocNode[], root = nodes[0].id): DocSnapshot {
  return {
    nodes: Object.fromEntries(nodes.map((node) => [String(node.id), node])),
    root,
    artboard: { x: 0, y: 0, w: 330, h: 3000 },
    machine: null,
  };
}

/** The fixture in `cutplan::passes::every_grouping_keys_the_same_shapes_differently`. */
function rustFixture(): DocSnapshot {
  const root = container(1, "Layer", [2, 3, 4, 5]);
  return doc([
    root,
    shape(2, { stroke: RED, fill: GREEN }),
    shape(3, { stroke: GREEN, fill: GREEN }),
    shape(4, { stroke: null, fill: BLUE }),
    shape(5, { stroke: null, fill: null }),
  ]);
}

describe("layer passes match the planner", () => {
  const keys = (grouping: Grouping) => layerPasses(rustFixture(), grouping).map((pass) => pass.key);

  test("every grouping keys the same shapes differently", () => {
    expect(keys("Single")).toEqual(["all"]);
    expect(keys("Color")).toEqual(["color:ff0000ff", "color:00ff00ff", "color:0000ffff", "no-color"]);
    expect(keys("Stroke")).toEqual(["color:ff0000ff", "color:00ff00ff", "no-color"]);
    expect(keys("Fill")).toEqual(["color:00ff00ff", "color:0000ffff", "no-color"]);
    expect(keys("Preset")).toEqual(["no-preset"]);

    for (const grouping of ["Single", "Color", "Stroke", "Fill", "Preset"] as const) {
      const shapes = layerPasses(rustFixture(), grouping).reduce((n, pass) => n + pass.shapeIds.length, 0);
      expect(shapes, grouping).toBe(4);
    }
  });

  test("a pass lists its cut shapes in document order", () => {
    expect(layerPasses(rustFixture(), "Single")[0].shapeIds).toEqual([2, 3, 4, 5]);
    expect(layerPasses(rustFixture(), "Fill").map((pass) => pass.shapeIds)).toEqual([[2, 3], [4], [5]]);
    expect(layerPasses(rustFixture(), "Stroke").find((pass) => pass.key === "no-color")?.shapeIds).toEqual([4, 5]);
  });

  test("preset grouping follows the resolved material, and unassigned stops the chain", () => {
    const root = container(1, "Layer", [2, 3]);
    const group = container(2, "Group", [4, 5], { material_preset: { state: "preset", id: "htv" } });
    const snapshot = doc([
      root,
      group,
      shape(3, { stroke: RED, fill: null }, { material_preset: { state: "unassigned" } }),
      shape(4, { stroke: RED, fill: null }),
      shape(5, { stroke: GREEN, fill: null }, { material_preset: { state: "preset", id: "card" } }),
    ]);
    expect(layerPasses(snapshot, "Preset")).toEqual([
      { key: "preset:htv", shapeIds: [4] },
      { key: "preset:card", shapeIds: [5] },
      { key: "no-preset", shapeIds: [3] },
    ]);
  });

  test("a NoCut shape stays on the block and out of the pass", () => {
    const root = container(1, "Layer", [2, 3]);
    const snapshot = doc([
      root,
      shape(2, { stroke: RED, fill: null }, { cut_line_type: "NoCut" }),
      shape(3, { stroke: RED, fill: null }),
    ]);
    expect(layerPasses(snapshot, "Color")).toEqual([{ key: "color:ff0000ff", shapeIds: [3] }]);
    expect(layerRowIds(snapshot, "Color")).toEqual([2, 3]);
  });

  test("an all-NoCut colour is a row and not a pass", () => {
    const root = container(1, "Layer", [2]);
    const snapshot = doc([root, shape(2, { stroke: RED, fill: null }, { cut_line_type: "NoCut" })]);
    expect(layerPasses(snapshot, "Color")).toEqual([]);
    expect(layerRowIds(snapshot, "Color")).toEqual([2]);
  });

  test("alpha 0 does not key a colour", () => {
    expect(shapePassKey("Color", { stroke: 0xff000000, fill: BLUE }, null)).toBe("color:0000ffff");
    expect(shapePassKey("Stroke", { stroke: 0xff000000, fill: BLUE }, null)).toBe("no-color");
  });
});

describe("layer row order", () => {
  test("a group and two colours stay in document preorder", () => {
    // seedGroup then seedTwoColorRects: Group, its rect, red, green.
    const root = container(1, "Layer", [2, 4, 5]);
    const group = container(2, "Group", [3]);
    const snapshot = doc([
      root,
      group,
      shape(3, { stroke: RED, fill: null }),
      shape(4, { stroke: RED, fill: null }),
      shape(5, { stroke: GREEN, fill: null }),
    ]);
    expect(layerRowIds(snapshot, "Color")).toEqual([2, 3, 4, 5]);
  });

  test("an empty group follows the shapes, as the align seed lays them out", () => {
    const root = container(1, "Layer", [2, 3, 4, 7]);
    const group = container(4, "Group", [5, 6]);
    const empty = container(7, "Group", []);
    const snapshot = doc([
      root,
      shape(2, { stroke: RED, fill: null }),
      shape(3, { stroke: GREEN, fill: null }),
      group,
      shape(5, { stroke: BLUE, fill: null }),
      shape(6, { stroke: BLUE, fill: null }),
      empty,
    ]);
    expect(layerRowIds(snapshot, "Color")).toEqual([2, 3, 4, 5, 6, 7]);
  });
});

describe("reorder index", () => {
  test("a drop uses the target's current index", () => {
    expect(applySiblingReorder(["A", "B", "C", "D"], 0, 2)).toEqual(["B", "C", "A", "D"]);
    expect(applySiblingReorder(["A", "B", "C", "D"], 3, 0)).toEqual(["D", "A", "B", "C"]);
    expect(applySiblingReorder(["A", "B", "C", "D"], 1, 1)).toBeNull();
  });
});
