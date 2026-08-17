// SPDX-License-Identifier: GPL-3.0-or-later
import type { DocNode } from "../App";
import type { PresetAssignmentJson } from "../ipc";

/// The local assignment a selection agrees on, `"mixed"` when it does not, or `undefined` when
/// nothing is selected.
///
/// Deliberately no descent, unlike `selectionCutLineType`: `commands::set_material_preset`
/// writes the selected Nodes themselves, because a material inherits and a Layer's own value is
/// what reaches the shapes under it. Reporting a child's value for a selected Layer would
/// describe something the control cannot write.
export function selectionAssignment(
  nodes: Record<string, DocNode>,
  selected: number[],
): PresetAssignmentJson | "mixed" | undefined {
  const seen = new Set<string>();
  let first: PresetAssignmentJson | undefined;
  for (const id of selected) {
    const node = nodes[String(id)];
    if (!node) continue;
    const assignment = node.material_preset;
    if (seen.size === 0) first = assignment;
    // Compared by serialization because the assignment is a tagged object, and two distinct
    // objects naming the same preset are the same answer to "what does this selection say".
    seen.add(JSON.stringify(assignment));
    if (seen.size > 1) return "mixed";
  }
  return first;
}

/// What each Node's material resolves to, keyed by id — the planner's rule, mirrored so the
/// panel can say "Inherited — HTV" without guessing. Nearest assigned ancestor wins, and
/// `unassigned` stops the chain instead of deferring up it.
export function effectiveMaterials(
  nodes: Record<string, DocNode>,
  root: number,
): Record<number, string | null> {
  const resolved: Record<number, string | null> = {};
  const seen = new Set<number>();
  const stack: [number, string | null][] = [[root, null]];
  while (stack.length > 0) {
    const [id, inherited] = stack.pop()!;
    // A malformed document whose nodes contain each other would otherwise spin here — the same
    // guard `plan_passes_with` keeps.
    if (seen.has(id)) continue;
    seen.add(id);
    const node = nodes[String(id)];
    if (!node) continue;
    const assignment = node.material_preset;
    const material =
      assignment.state === "preset" ? assignment.id
      : assignment.state === "unassigned" ? null
      : inherited;
    resolved[id] = material;
    for (const child of node.children) stack.push([child, material]);
  }
  return resolved;
}

/// What a selection's material resolves to: one value — an id, or `null` for no material — or
/// more than one, when the selected Nodes inherit from ancestors that disagree. Tagged rather
/// than a reserved string, because a preset id is the operator's own and one called `mixed`
/// would collide with the marker.
export type EffectiveMaterial = { kind: "one"; id: string | null } | { kind: "mixed" };

/// Reduce a whole selection to one answer. Every selected Node, never just the first: two shapes
/// that both say `Inherit` under different Layers agree on their *local* value and resolve
/// differently, and labelling the pair with one of them misreports what a bulk edit replaces.
export function summariseEffectiveMaterial(
  materialsByNode: Record<number, string | null>,
  selected: number[],
): EffectiveMaterial {
  if (selected.length === 0) return { kind: "one", id: null };
  const distinct = new Set(selected.map((id) => materialsByNode[id] ?? null));
  if (distinct.size > 1) return { kind: "mixed" };
  return { kind: "one", id: [...distinct][0] ?? null };
}
