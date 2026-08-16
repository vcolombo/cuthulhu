// SPDX-License-Identifier: GPL-3.0-or-later
import type { Bounds } from "../render/hittest";
import type { CutLineTypeJson } from "./cutLineType";
import { NumberField } from "./NumberField";

type Props = {
  bounds: Bounds | null;
  cutLineType: CutLineTypeJson | "mixed" | null;
  onChangeX: (v: number) => void;
  onChangeY: (v: number) => void;
  onChangeW: (v: number) => void;
  onChangeH: (v: number) => void;
  onChangeCutLineType: (v: CutLineTypeJson) => void;
};

export function PropertiesPanel({ bounds, cutLineType, onChangeX, onChangeY, onChangeW, onChangeH,
                                  onChangeCutLineType }: Props) {
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
      ) : null}
      {/* Outside the `bounds` branch: `selectedBounds` is null for every multi-node selection
          and for a selected container (App.tsx), both of which do have a cuttability. */}
      {cutLineType !== null ? (
        <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
          <input
            type="checkbox"
            aria-label="Cut this shape"
            checked={cutLineType === "Cut"}
            // A mixed selection shows the browser's indeterminate mark rather than picking a
            // side; clicking it commits `Cut` for everything, which is the recoverable
            // direction (one undo, or one more click).
            ref={(el) => { if (el) el.indeterminate = cutLineType === "mixed"; }}
            onChange={(e) => onChangeCutLineType(e.target.checked ? "Cut" : "NoCut")}
          />
          Cut
        </label>
      ) : null}
      {bounds === null && cutLineType === null ? (
        <div style={{ fontSize: 12, color: "var(--muted)" }}>No selection</div>
      ) : null}
    </div>
  );
}
