// SPDX-License-Identifier: GPL-3.0-or-later
import type { DocNode, DocSnapshot, NodeKindJson, StyleJson } from "../App";
import type { Grouping } from "../ipc";
import { passRowLabel, type PresetLookup } from "../cut/viewmodel";

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
  /** NoCut shapes of the same key, so the row can be turned back on. */
  heldIds: number[];
  /** Every shape in the block, document order. */
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

/** Alpha 0 is absent, the same rule `cutplan::passes::color_key` uses. */
function visibleChannel(color: number | null): number | null {
  if (color === null) return null;
  if ((color & 0xff) === 0) return null;
  return color >>> 0;
}

/** The pass key a shape would take under `grouping`. Mirrors `plan_passes_with`. */
export function shapePassKey(grouping: Grouping, style: StyleJson, material: string | null): string {
  if (grouping === "Single") return "all";
  if (grouping === "Preset") return material === null ? "no-preset" : `preset:${material}`;
  const stroke = visibleChannel(style.stroke);
  const fill = visibleChannel(style.fill);
  const color = grouping === "Stroke" ? stroke : grouping === "Fill" ? fill : stroke ?? fill;
  if (color === null) return "no-color";
  return `color:${color.toString(16).padStart(8, "0")}`;
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

/**
 * Colour blocks in first-seen pass order, plus containers the planner never emits.
 * Cut ids inside a block are `plan_passes_with` for that key: a NoCut shape is held on the
 * block so the toggle can turn the row back on, and it is not part of the pass.
 */
export function layerModel(doc: DocSnapshot, grouping: Grouping, lookup: PresetLookup = EMPTY_LOOKUP): LayerModel {
  const names = defaultNames(doc);
  const parents = parentOf(doc);
  const seen = new Set<number>();
  const tagged: Tagged[] = [];

  const walk = (id: number, inherited: string | null) => {
    if (seen.has(id)) return;
    seen.add(id);
    const node = doc.nodes[String(id)];
    if (!node) return;
    const assignment = node.material_preset;
    const material = assignment.state === "preset" ? assignment.id
      : assignment.state === "unassigned" ? null
      : inherited;
    if (isShape(node)) {
      tagged.push({
        id,
        cut: node.cut_line_type !== "NoCut",
        key: shapePassKey(grouping, node.style, material),
      });
    }
    for (const child of node.children) walk(child, material);
  };
  walk(doc.root, null);

  const order: string[] = [];
  const byKey = new Map<string, Tagged[]>();
  for (const shape of tagged) {
    const list = byKey.get(shape.key);
    if (list) list.push(shape);
    else {
      order.push(shape.key);
      byKey.set(shape.key, [shape]);
    }
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

  const blocks: LayerBlock[] = order.map((key) => {
    const shapes = byKey.get(key) ?? [];
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
    const heldIds = shapes.filter((shape) => !shape.cut).map((shape) => shape.id);
    const cut = cutIds.length === 0 ? "NoCut" : heldIds.length === 0 ? "Cut" : "mixed";
    return {
      key,
      name: label.text ?? key,
      swatch: label.swatch,
      cutIds,
      heldIds,
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

/** The planner's passes: cut shapes only, first-seen key order. NoCut-only blocks are not passes. */
export function layerPasses(doc: DocSnapshot, grouping: Grouping): { key: string; shapeIds: number[] }[] {
  return layerModel(doc, grouping).blocks
    .filter((block) => block.cutIds.length > 0)
    .map((block) => ({ key: block.key, shapeIds: block.cutIds }));
}

/** Every `layer-row` id, in the order the panel lists them. */
export function layerRowIds(doc: DocSnapshot, grouping: Grouping): number[] {
  const model = layerModel(doc, grouping);
  return [...model.blocks.flatMap((block) => block.rows.map((row) => row.id)), ...model.trailing.map((row) => row.id)];
}

/**
 * `reorder`'s `newIndex` is the target sibling's current index. Remove-then-insert at that index
 * matches a splice: moving down lands after the target, moving up takes its place.
 * Null when the drop would not move the node.
 */
export function reorderNewIndex(from: number, to: number): number | null {
  if (!Number.isInteger(from) || !Number.isInteger(to)) return null;
  if (from === to || from < 0 || to < 0) return null;
  return to;
}

/** Where a drop of `dragId` onto `targetId` lands, or null when they are not siblings or nothing moves. */
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
  const next = reorderNewIndex(siblings.indexOf(dragId), siblings.indexOf(targetId));
  if (next === null) return null;
  return { id: dragId, newIndex: next };
}

/** What `reorder` does to a sibling list. Used to pin the index rule. */
export function applySiblingReorder<T>(items: T[], from: number, to: number): T[] | null {
  const index = reorderNewIndex(from, to);
  if (index === null || from >= items.length || to >= items.length) return null;
  const next = items.slice();
  const [item] = next.splice(from, 1);
  next.splice(index, 0, item);
  return next;
}
