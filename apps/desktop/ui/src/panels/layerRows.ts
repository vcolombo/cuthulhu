// SPDX-License-Identifier: GPL-3.0-or-later
import type { DocNode, DocSnapshot, NodeKindJson } from "../App";
import type { Grouping } from "../ipc";
import { passKeyFor, passRowLabel, type PresetLookup } from "../cut/viewmodel";

/** What a row's icon is. Containers stay "Group" / "Layer" so the existing panel queries still match. */
export type LayerIcon = "rectangle" | "ellipse" | "text" | "path" | "group" | "layer";

export type LayerNodeRow = {
  id: number;
  name: string;
  icon: LayerIcon;
  depth: number;
};

export type LayerBlock = {
  key: string;
  name: string;
  /** `#rrggbb` for `vinylDisplay`, or null when the pass has no colour (no paint, one pass, a preset). */
  swatch: string | null;
  /** Cut shapes, document order. This is the planner's pass. */
  cutIds: number[];
  /** Every shape in the block, document order. NoCut shapes stay so the row can be turned back on. */
  shapeIds: number[];
  /** "Cut" when every shape cuts, "NoCut" when none does. */
  cut: "Cut" | "NoCut" | "mixed";
  rows: LayerNodeRow[];
};

export type LayerModel = {
  blocks: LayerBlock[];
  /** Containers that hold no shape, after the colour blocks. They are not passes. */
  trailing: LayerNodeRow[];
};

const EMPTY_LOOKUP: PresetLookup = { presets: [], loaded: false };

function isShape(node: DocNode): boolean {
  return typeof node.kind === "object" && node.kind !== null && "Shape" in node.kind;
}

export function kindLabel(kind: NodeKindJson): "Rectangle" | "Ellipse" | "Text" | "Path" | "Group" | "Layer" {
  if (kind === "Layer") return "Layer";
  if (kind === "Group") return "Group";
  const shape = kind.Shape;
  if ("Rect" in shape) return "Rectangle";
  if ("Ellipse" in shape) return "Ellipse";
  if ("Text" in shape) return "Text";
  return "Path";
}

function iconFor(kind: NodeKindJson): LayerIcon {
  const label = kindLabel(kind);
  if (label === "Rectangle") return "rectangle";
  if (label === "Ellipse") return "ellipse";
  if (label === "Text") return "text";
  if (label === "Path") return "path";
  if (label === "Group") return "group";
  return "layer";
}

/** 1-based per shape kind, in document preorder ("Rectangle 3"). Containers keep the bare word. */
export function defaultNames(doc: DocSnapshot): Map<number, string> {
  const count = { Rectangle: 0, Ellipse: 0, Text: 0, Path: 0 };
  const names = new Map<number, string>();
  const walk = (id: number) => {
    const node = doc.nodes[String(id)];
    if (!node) return;
    if (id !== doc.root) {
      const label = kindLabel(node.kind);
      if (label === "Group" || label === "Layer") names.set(id, label);
      else {
        count[label] += 1;
        names.set(id, `${label} ${count[label]}`);
      }
    }
    for (const child of node.children) walk(child);
  };
  walk(doc.root);
  return names;
}

function parentOf(doc: DocSnapshot): Map<number, number> {
  const parents = new Map<number, number>();
  for (const node of Object.values(doc.nodes)) {
    for (const child of node.children) parents.set(child, node.id);
  }
  return parents;
}

function depthOf(parents: Map<number, number>, id: number, root: number): number {
  let depth = 0;
  let cur = id;
  for (;;) {
    const parent = parents.get(cur);
    if (parent === undefined || parent === root) break;
    depth += 1;
    cur = parent;
  }
  return depth;
}

function ancestorChain(parents: Map<number, number>, id: number, root: number): number[] {
  const chain: number[] = [];
  let cur = parents.get(id);
  while (cur !== undefined && cur !== root) {
    chain.push(cur);
    cur = parents.get(cur);
  }
  chain.reverse();
  return chain;
}

type Tagged = { id: number; cut: boolean; key: string };

/** Shapes in document preorder. Material comes from `effectiveMaterials`; this walk only places them. */
function taggedShapes(
  doc: DocSnapshot,
  grouping: Grouping,
  materials: Readonly<Record<number, string | null>>,
): Tagged[] {
  const seen = new Set<number>();
  const tagged: Tagged[] = [];
  const walk = (id: number) => {
    if (seen.has(id)) return;
    seen.add(id);
    const node = doc.nodes[String(id)];
    if (!node) return;
    if (isShape(node)) {
      tagged.push({
        id,
        cut: node.cut_line_type !== "NoCut",
        key: passKeyFor(grouping, node.style, materials[id] ?? null),
      });
    }
    for (const child of node.children) walk(child);
  };
  walk(doc.root);
  return tagged;
}

function groupInOrder(shapes: Tagged[]): { order: string[]; byKey: Map<string, Tagged[]> } {
  const order: string[] = [];
  const byKey = new Map<string, Tagged[]>();
  for (const shape of shapes) {
    const list = byKey.get(shape.key);
    if (list) list.push(shape);
    else {
      order.push(shape.key);
      byKey.set(shape.key, [shape]);
    }
  }
  return { order, byKey };
}

/** The planner's passes: cut shapes only, first-seen key order. NoCut-only keys are not passes. */
export function layerPasses(
  doc: DocSnapshot,
  grouping: Grouping,
  materials: Readonly<Record<number, string | null>>,
): { key: string; shapeIds: number[] }[] {
  const { order, byKey } = groupInOrder(taggedShapes(doc, grouping, materials).filter((shape) => shape.cut));
  return order.map((key) => ({ key, shapeIds: (byKey.get(key) ?? []).map((shape) => shape.id) }));
}

/**
 * Colour blocks in the planner's pass order, plus a NoCut-only key where that colour first
 * appears, plus containers the planner never emits. NoCut shapes of a key that does cut stay
 * on that block so the toggle can turn the row back on.
 */
export function layerModel(
  doc: DocSnapshot,
  grouping: Grouping,
  materials: Readonly<Record<number, string | null>>,
  lookup: PresetLookup = EMPTY_LOOKUP,
): LayerModel {
  const names = defaultNames(doc);
  const parents = parentOf(doc);
  const tagged = taggedShapes(doc, grouping, materials);
  const all = groupInOrder(tagged);
  const cuts = groupInOrder(tagged.filter((shape) => shape.cut));
  const firstAt = new Map<string, number>();
  const firstCutAt = new Map<string, number>();
  tagged.forEach((shape, index) => {
    if (!firstAt.has(shape.key)) firstAt.set(shape.key, index);
    if (shape.cut && !firstCutAt.has(shape.key)) firstCutAt.set(shape.key, index);
  });
  const keys = [...cuts.order];
  for (const key of all.order) {
    if (cuts.byKey.has(key)) continue;
    const at = firstAt.get(key) ?? 0;
    const insert = keys.findIndex((cutKey) => (firstCutAt.get(cutKey) ?? 0) > at);
    keys.splice(insert === -1 ? keys.length : insert, 0, key);
  }

  const emitted = new Set<number>();
  const rowFor = (id: number): LayerNodeRow => {
    const node = doc.nodes[String(id)];
    return {
      id,
      name: names.get(id) ?? "Path",
      icon: node ? iconFor(node.kind) : "path",
      depth: depthOf(parents, id, doc.root),
    };
  };

  const blocks: LayerBlock[] = keys.map((key) => {
    const shapes = all.byKey.get(key) ?? [];
    const label = passRowLabel(key, lookup, grouping);
    const rows: LayerNodeRow[] = [];
    for (const shape of shapes) {
      for (const anc of ancestorChain(parents, shape.id, doc.root)) {
        if (emitted.has(anc)) continue;
        emitted.add(anc);
        rows.push(rowFor(anc));
      }
      if (!emitted.has(shape.id)) {
        emitted.add(shape.id);
        rows.push(rowFor(shape.id));
      }
    }
    const cutIds = shapes.filter((shape) => shape.cut).map((shape) => shape.id);
    const cut = cutIds.length === 0 ? "NoCut" : cutIds.length === shapes.length ? "Cut" : "mixed";
    return {
      key,
      name: label.text ?? key,
      swatch: label.swatch,
      cutIds,
      shapeIds: shapes.map((shape) => shape.id),
      cut,
      rows,
    };
  });

  const trailing: LayerNodeRow[] = [];
  const walkTrailing = (id: number) => {
    const node = doc.nodes[String(id)];
    if (!node) return;
    if (id !== doc.root && !isShape(node) && !emitted.has(id)) {
      emitted.add(id);
      trailing.push(rowFor(id));
    }
    if (!isShape(node)) for (const child of node.children) walkTrailing(child);
  };
  walkTrailing(doc.root);

  return { blocks, trailing };
}

/** Every `layer-row` id, in the order the panel lists them. */
export function layerRowIds(
  doc: DocSnapshot,
  grouping: Grouping,
  materials: Readonly<Record<number, string | null>>,
): number[] {
  const model = layerModel(doc, grouping, materials);
  return [...model.blocks.flatMap((block) => block.rows.map((row) => row.id)), ...model.trailing.map((row) => row.id)];
}

/** Where a drop of `dragId` onto `targetId` lands. `newIndex` is the target sibling's current
 *  index, which is what `reorder` removes-then-inserts at (`reorderTo` is that splice).
 *  Null when they are not siblings or nothing moves. */
export function reorderTarget(
  doc: DocSnapshot,
  dragId: number,
  targetId: number,
): { id: number; newIndex: number } | null {
  if (dragId === targetId) return null;
  const parents = parentOf(doc);
  const parent = parents.get(dragId);
  if (parent === undefined || parent !== parents.get(targetId)) return null;
  const siblings = doc.nodes[String(parent)]?.children;
  if (!siblings) return null;
  const from = siblings.indexOf(dragId);
  const to = siblings.indexOf(targetId);
  if (from < 0 || to < 0 || from === to) return null;
  return { id: dragId, newIndex: to };
}
