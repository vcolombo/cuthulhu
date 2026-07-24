// SPDX-License-Identifier: GPL-3.0-or-later
import type { Bounds } from "../render/hittest";
import { NumberField } from "./NumberField";

type Props = {
  bounds: Bounds | null;
  onChangeX: (v: number) => void;
  onChangeY: (v: number) => void;
  onChangeW: (v: number) => void;
  onChangeH: (v: number) => void;
};

export function PropertiesPanel({ bounds, onChangeX, onChangeY, onChangeW, onChangeH }: Props) {
  return (
    <div style={{ padding: 8, display: "flex", flexDirection: "column", gap: 6, overflowY: "auto" }}>
      <div style={{ fontSize: 11, color: "var(--muted)", textTransform: "uppercase" }}>Properties</div>
      {bounds ? (
        <>
          <NumberField label="X" value={bounds.x} onChange={onChangeX} />
          <NumberField label="Y" value={bounds.y} onChange={onChangeY} />
          <NumberField label="W" value={bounds.w} min={0} onChange={onChangeW} />
          <NumberField label="H" value={bounds.h} min={0} onChange={onChangeH} />
        </>
      ) : (
        <div style={{ fontSize: 12, color: "var(--muted)" }}>No selection</div>
      )}
    </div>
  );
}
