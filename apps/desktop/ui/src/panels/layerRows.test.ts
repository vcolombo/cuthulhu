// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, test } from "vitest";
import type { DocNode, DocSnapshot } from "../App";
import type { Grouping } from "../ipc";
import { effectiveMaterials } from "./materialPreset";
import { documentPasses, layerFooter, layerHeading, layerModel, layerRowIds, reorderTarget } from "./layerRows";

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

function passes(snapshot: DocSnapshot, grouping: Grouping) {
  return documentPasses(snapshot, grouping, effectiveMaterials(snapshot.nodes, snapshot.root));
}

function rowIds(snapshot: DocSnapshot, grouping: Grouping) {
  return layerRowIds(snapshot, grouping, effectiveMaterials(snapshot.nodes, snapshot.root));
}

describe("layer passes match the planner", () => {
  const keys = (grouping: Grouping) => passes(rustFixture(), grouping).map((pass) => pass.key);

  test("every grouping keys the same shapes differently", () => {
    expect(keys("Single")).toEqual(["all"]);
    expect(keys("Color")).toEqual(["color:ff0000ff", "color:00ff00ff", "color:0000ffff", "no-color"]);
    expect(keys("Stroke")).toEqual(["color:ff0000ff", "color:00ff00ff", "no-color"]);
    expect(keys("Fill")).toEqual(["color:00ff00ff", "color:0000ffff", "no-color"]);
    expect(keys("Preset")).toEqual(["no-preset"]);

    for (const grouping of ["Single", "Color", "Stroke", "Fill", "Preset"] as const) {
      const shapes = passes(rustFixture(), grouping).reduce((n, pass) => n + pass.shapeIds.length, 0);
      expect(shapes, grouping).toBe(4);
    }
  });

  test("a pass lists its cut shapes in document order", () => {
    expect(passes(rustFixture(), "Single")[0].shapeIds).toEqual([2, 3, 4, 5]);
    expect(passes(rustFixture(), "Fill").map((pass) => pass.shapeIds)).toEqual([[2, 3], [4], [5]]);
    expect(passes(rustFixture(), "Stroke").find((pass) => pass.key === "no-color")?.shapeIds).toEqual([4, 5]);
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
    expect(passes(snapshot, "Preset")).toEqual([
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
    expect(passes(snapshot, "Color")).toEqual([{ key: "color:ff0000ff", shapeIds: [3] }]);
    expect(rowIds(snapshot, "Color")).toEqual([2, 3]);
  });

  test("an all-NoCut colour is a row and not a pass", () => {
    const root = container(1, "Layer", [2]);
    const snapshot = doc([root, shape(2, { stroke: RED, fill: null }, { cut_line_type: "NoCut" })]);
    expect(passes(snapshot, "Color")).toEqual([]);
    expect(rowIds(snapshot, "Color")).toEqual([2]);
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
    expect(rowIds(snapshot, "Color")).toEqual([2, 3, 4, 5]);
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
    expect(rowIds(snapshot, "Color")).toEqual([2, 3, 4, 5, 6, 7]);
  });

  test("an empty group nested under a group stays in that group's block", () => {
    const root = container(1, "Layer", [2]);
    const group = container(2, "Group", [3, 8]);
    const empty = container(8, "Group", []);
    const snapshot = doc([
      root,
      group,
      shape(3, { stroke: RED, fill: null }),
      empty,
    ]);
    const model = layerModel(snapshot, "Color", effectiveMaterials(snapshot.nodes, snapshot.root));
    expect(model.blocks.map((block) => block.rows.map((row) => row.id))).toEqual([[2, 3, 8]]);
    expect(model.trailing).toEqual([]);
  });

  test("an empty group between two colours of its parent stays with the earlier block", () => {
    const root = container(1, "Layer", [2]);
    const group = container(2, "Group", [3, 8, 4]);
    const empty = container(8, "Group", []);
    const snapshot = doc([
      root,
      group,
      shape(3, { stroke: RED, fill: null }),
      empty,
      shape(4, { stroke: GREEN, fill: null }),
    ]);
    const model = layerModel(snapshot, "Color", effectiveMaterials(snapshot.nodes, snapshot.root));
    expect(model.blocks.map((block) => block.rows.map((row) => [row.id, row.repeated]))).toEqual([
      [[2, false], [3, false], [8, false]],
      [[2, true], [4, false]],
    ]);
    expect(model.trailing).toEqual([]);
  });

  test("a group whose shapes span colours is repeated, dimmed, under the later block", () => {
    const root = container(1, "Layer", [2]);
    const group = container(2, "Group", [3, 4]);
    const snapshot = doc([
      root,
      group,
      shape(3, { stroke: RED, fill: null }),
      shape(4, { stroke: GREEN, fill: null }),
    ]);
    const model = layerModel(snapshot, "Color", effectiveMaterials(snapshot.nodes, snapshot.root));
    expect(model.blocks.map((block) => block.rows.map((row) => [row.id, row.repeated, row.depth]))).toEqual([
      [[2, false, 0], [3, false, 1]],
      [[2, true, 0], [4, false, 1]],
    ]);
  });
});

describe("layer copy", () => {
  test("colour groupings name colour layers, and the footer never calls a drag a pass reorder", () => {
    for (const grouping of ["Color", "Stroke", "Fill"] as const) {
      expect(layerHeading(grouping)).toBe("Colour layers");
      expect(layerFooter(grouping)).not.toContain("cut order");
    }
    expect(layerHeading("Single")).toBe("Layers");
    expect(layerHeading("Preset")).toBe("Materials");
    expect(layerFooter("Single")).toContain("one pass");
    expect(layerFooter("Preset")).toContain("material");
    expect(layerFooter("Color")).not.toContain("material");
    expect(layerFooter("Preset")).not.toContain("colour");
    expect(layerFooter("Single")).not.toContain("colour");
  });
});

describe("reorder index", () => {
  test("a drop names the target sibling's current index", () => {
    const root = container(1, "Layer", [2, 3, 4, 5]);
    const snapshot = doc([
      root,
      shape(2, { stroke: RED, fill: null }),
      shape(3, { stroke: RED, fill: null }),
      shape(4, { stroke: GREEN, fill: null }),
      shape(5, { stroke: GREEN, fill: null }),
    ]);
    expect(reorderTarget(snapshot, 2, 4)).toEqual({ id: 2, newIndex: 2 });
    expect(reorderTarget(snapshot, 5, 3)).toEqual({ id: 5, newIndex: 1 });
    expect(reorderTarget(snapshot, 2, 2)).toBeNull();
    expect(reorderTarget(snapshot, 3, 1)).toBeNull();
  });
});
