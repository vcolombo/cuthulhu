// SPDX-License-Identifier: GPL-3.0-or-later
import type { Bounds } from "../render/hittest";
import { NumberField } from "./NumberField";

type Props = {
  bounds: Bounds | null;
  onChangeX: (v: number) => void;
  onChangeY: (v: number) => void;
};

export function PropertiesPanel({ bounds, onChangeX, onChangeY }: Props) {
  return (
    <div style={{ padding: 8, display: "flex", flexDirection: "column", gap: 6, overflowY: "auto" }}>
      <div style={{ fontSize: 11, color: "var(--muted)", textTransform: "uppercase" }}>Properties</div>
      {bounds ? (
        <>
          <NumberField label="X" value={bounds.x} onChange={onChangeX} />
          <NumberField label="Y" value={bounds.y} onChange={onChangeY} />
          {/* ponytail: W/H shown read-only — no backend resize command yet, and scaling via
              commit_transform's world-space-after-existing-transform composition would also
              move the shape's origin unless offset, which is not "thin wiring". Add a
              resize_node command, then wire these live. */}
          <NumberField label="W" value={bounds.w} disabled onChange={() => {}} />
          <NumberField label="H" value={bounds.h} disabled onChange={() => {}} />
        </>
      ) : (
        <div style={{ fontSize: 12, color: "var(--muted)" }}>No selection</div>
      )}
    </div>
  );
}
