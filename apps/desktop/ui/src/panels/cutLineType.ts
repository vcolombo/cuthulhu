// SPDX-License-Identifier: GPL-3.0-or-later
import type { DocNode } from "../App";

export type CutLineTypeJson = "Cut" | "NoCut";

/// What to show for a selection: one value, "mixed" when the shapes disagree, or null when
/// there is no shape to speak for. Mirrors `commands::set_cut_line_type`, which walks into
/// containers because the attribute is read only on shapes — a panel that read the selected
/// node's own value would show a Group's inert one.
export function selectionCutLineType(
  nodes: Record<string, DocNode>,
  selected: number[],
): CutLineTypeJson | "mixed" | null {
  const values = new Set<CutLineTypeJson>();
  const seen = new Set<number>();
  const stack = [...selected];
  while (stack.length > 0) {
    const id = stack.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    const node = nodes[String(id)];
    if (!node) continue;
    if (typeof node.kind === "object" && "Shape" in node.kind) values.add(node.cut_line_type);
    else stack.push(...node.children);
  }
  if (values.size === 0) return null;
  return values.size === 1 ? [...values][0] : "mixed";
}
