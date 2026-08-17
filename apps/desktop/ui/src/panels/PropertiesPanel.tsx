// SPDX-License-Identifier: GPL-3.0-or-later
import type { Bounds } from "../render/hittest";
import type { CutLineTypeJson } from "./cutLineType";
import type { PresetAssignmentJson } from "../ipc";
import type { Preset } from "../cut/viewmodel";
import { NumberField } from "./NumberField";

/** What a selection's material resolves to: one value — an id, or `null` for no material — or
 *  more than one, when the selected Nodes inherit from ancestors that disagree. Tagged rather
 *  than a reserved string, because a preset id is the operator's own and one called `mixed`
 *  would collide with the marker. */
export type EffectiveMaterial = { kind: "one"; id: string | null } | { kind: "mixed" };

type Props = {
  bounds: Bounds | null;
  cutLineType: CutLineTypeJson | "mixed" | null;
  /** The selection's own assignment; `undefined` when there is no selection. */
  materialPreset: PresetAssignmentJson | "mixed" | undefined;
  effectiveMaterial: EffectiveMaterial;
  presets: Preset[];
  onChangeX: (v: number) => void;
  onChangeY: (v: number) => void;
  onChangeW: (v: number) => void;
  onChangeH: (v: number) => void;
  onChangeCutLineType: (v: CutLineTypeJson) => void;
  onChangeMaterialPreset: (v: PresetAssignmentJson) => void;
};

/** What the material row reads for each state. `Inherit` shows what it resolves to, because
 *  "inherit" alone does not tell an operator which material the blade will be set for. */
function materialLabel(
  assignment: PresetAssignmentJson | "mixed",
  effective: EffectiveMaterial,
  presets: Preset[],
): string {
  const name = (id: string) => presets.find((p) => p.id === id)?.name ?? `Unresolved (${id})`;
  if (assignment === "mixed") return "Mixed";
  switch (assignment.state) {
    case "preset": return name(assignment.id);
    case "unassigned": return "No preset";
    case "inherit":
      return effective.kind === "mixed" ? "Inherited — Mixed"
           : effective.id === null ? "Inherited — No preset"
           : `Inherited — ${name(effective.id)}`;
  }
}

export function PropertiesPanel({ bounds, cutLineType, materialPreset, effectiveMaterial, presets,
                                  onChangeX, onChangeY, onChangeW, onChangeH,
                                  onChangeCutLineType, onChangeMaterialPreset }: Props) {
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
      {materialPreset !== undefined ? (
        <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
          Material
          <select
            aria-label="Material preset"
            value={materialPreset === "mixed" ? ""
                 : materialPreset.state === "preset" ? `preset:${materialPreset.id}`
                 : materialPreset.state}
            onChange={(e) => {
              const v = e.target.value;
              onChangeMaterialPreset(
                v === "inherit" ? { state: "inherit" }
                : v === "unassigned" ? { state: "unassigned" }
                : { state: "preset", id: v.slice("preset:".length) },
              );
            }}
          >
            {/* A mixed selection shows this inert option as selected rather than picking a
                side; every other option commits, which one undo reverses. */}
            {materialPreset === "mixed" ? <option value="" disabled>Mixed</option> : null}
            <option value="inherit">Inherit</option>
            <option value="unassigned">No preset</option>
            {presets.map((p) => (
              <option key={p.id} value={`preset:${p.id}`}>{p.name}</option>
            ))}
          </select>
          <span style={{ color: "var(--muted)" }}>
            {materialLabel(materialPreset, effectiveMaterial, presets)}
          </span>
        </label>
      ) : null}
      {bounds === null && cutLineType === null && materialPreset === undefined ? (
        <div style={{ fontSize: 12, color: "var(--muted)" }}>No selection</div>
      ) : null}
    </div>
  );
}
