// SPDX-License-Identifier: GPL-3.0-or-later
import type { DocNode, DocSnapshot, NodeKindJson } from "../App";

type Props = {
  doc: DocSnapshot | null;
  selected: number[];
  onSelect: (id: number, shiftKey: boolean) => void;
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

function Row({ doc, node, depth, selected, onSelect }: { doc: DocSnapshot; node: DocNode; depth: number; selected: number[]; onSelect: (id: number, shiftKey: boolean) => void }) {
  const isSelected = selected.includes(node.id);
  return (
    <>
      <div
        data-testid="layer-row"
        onClick={(e) => onSelect(node.id, e.shiftKey)}
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
      {/* The document root is an implicit container (Document::new() always creates it) —
          only its children are user-visible layers, so it doesn't get its own row. */}
      {doc && root
        ? root.children.map((childId) => {
            const child = doc.nodes[childId];
            return child ? <Row key={childId} doc={doc} node={child} depth={0} selected={selected} onSelect={onSelect} /> : null;
          })
        : null}
    </div>
  );
}
