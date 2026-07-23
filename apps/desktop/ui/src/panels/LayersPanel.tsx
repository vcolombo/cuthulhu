// SPDX-License-Identifier: GPL-3.0-or-later
import type { DocNode, DocSnapshot, NodeKindJson } from "../App";

type Props = {
  doc: DocSnapshot | null;
  selected: number[];
  onSelect: (id: number) => void;
};

function labelFor(kind: NodeKindJson): string {
  if (kind === "Layer") return "Layer";
  if (kind === "Group") return "Group";
  const shape = kind.Shape;
  if ("Rect" in shape) return "Rectangle";
  if ("Ellipse" in shape) return "Ellipse";
  if ("Text" in shape) return "Text";
  return "Path";
}

function Row({ doc, node, depth, selected, onSelect }: { doc: DocSnapshot; node: DocNode; depth: number; selected: number[]; onSelect: (id: number) => void }) {
  const isSelected = selected.includes(node.id);
  return (
    <>
      <div
        data-testid="layer-row"
        onClick={() => onSelect(node.id)}
        style={{
          padding: `4px 8px 4px ${8 + depth * 12}px`,
          background: isSelected ? "var(--accent)" : "transparent",
          color: "var(--text)",
          fontSize: 12,
          cursor: "pointer",
        }}
      >
        {labelFor(node.kind)}
      </div>
      {node.children.map((childId) => {
        const child = doc.nodes[childId];
        return child ? <Row key={childId} doc={doc} node={child} depth={depth + 1} selected={selected} onSelect={onSelect} /> : null;
      })}
    </>
  );
}

export function LayersPanel({ doc, selected, onSelect }: Props) {
  const root = doc?.nodes[doc.root];
  return (
    <div style={{ overflowY: "auto", borderBottom: "1px solid var(--border)" }}>
      <div style={{ padding: "4px 8px", fontSize: 11, color: "var(--muted)", textTransform: "uppercase" }}>Layers</div>
      {doc && root ? <Row doc={doc} node={root} depth={0} selected={selected} onSelect={onSelect} /> : null}
    </div>
  );
}
