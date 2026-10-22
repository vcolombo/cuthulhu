// SPDX-License-Identifier: GPL-3.0-or-later
import type { MachineProfile } from "../App";
import type { Bounds } from "../render/hittest";
import type { CutLineTypeJson } from "./cutLineType";
import type { PresetAssignmentJson } from "../ipc";
import type { Preset } from "../cut/viewmodel";
import type { EffectiveMaterial } from "./materialPreset";
import { NumberField } from "./NumberField";
import { AlignRow } from "./AlignRow";
import type { AlignMode, Axis, DistributeBlock } from "../interaction/align";

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
  unitCount: number;
  /** Why edits are refused for now, or null; see `CanvasInteraction.editsLock`. */
  editsLocked: string | null;
  distributeBlocked: Record<Axis, DistributeBlock | null>;
  onAlign: (mode: AlignMode) => void;
  onDistribute: (axis: Axis) => void;
  onChangeCutLineType: (v: CutLineTypeJson) => void;
  onChangeMaterialPreset: (v: PresetAssignmentJson) => void;
  /** Document machine, chosen without connecting a cutter. Always shown: a selection still has a page. */
  machines?: MachineProfile[];
  machineId?: string | null;
  onDesignFor?: (machineId: string) => void;
};

/** What the material row reads for each state. Exported for its own test: it is the consumer
 *  that has to keep a preset called `mixed` apart from a mixed selection. `Inherit` shows what it resolves to, because
 *  "inherit" alone does not tell an operator which material the blade will be set for. */
export function materialLabel(
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
                                  onChangeX, onChangeY, onChangeW, onChangeH, unitCount, editsLocked, distributeBlocked, onAlign, onDistribute,
                                  onChangeCutLineType, onChangeMaterialPreset, machines, machineId, onDesignFor }: Props) {
  return (
    <div style={{ padding: 8, display: "flex", flexDirection: "column", gap: 6, overflowY: "auto" }}>
      {onDesignFor ? (
        <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12 }}>
          Design for
          <select
            aria-label="Design for"
            value={machineId ?? ""}
            onChange={(e) => {
              if (e.target.value) onDesignFor(e.target.value);
            }}
          >
            <option value="" disabled>
              Select machine
            </option>
            {(machines ?? []).map((machine) => (
              <option key={machine.id} value={machine.id}>
                {machine.name}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      <div style={{ fontSize: 11, color: "var(--muted-foreground)", textTransform: "uppercase" }}>Properties</div>
      {bounds ? (
        <>
          <NumberField label="X" value={bounds.x} disabled={editsLocked !== null} onChange={onChangeX} />
          <NumberField label="Y" value={bounds.y} disabled={editsLocked !== null} onChange={onChangeY} />
          <NumberField label="W" value={bounds.w} min={0} disabled={editsLocked !== null} onChange={onChangeW} />
          <NumberField label="H" value={bounds.h} min={0} disabled={editsLocked !== null} onChange={onChangeH} />
        </>
      ) : null}
      {/* Also outside it, since aligning is mostly for multi-node selections. Always shown, disabled
          when it cannot act, so the buttons do not jump the rows below them in and out. */}
      <AlignRow unitCount={unitCount} locked={editsLocked} distributeBlocked={distributeBlocked} onAlign={onAlign} onDistribute={onDistribute} />
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
          <span style={{ color: "var(--muted-foreground)" }}>
            {materialLabel(materialPreset, effectiveMaterial, presets)}
          </span>
        </label>
      ) : null}
      {bounds === null && cutLineType === null && materialPreset === undefined ? (
        <div style={{ fontSize: 12, color: "var(--muted-foreground)" }}>No selection</div>
      ) : null}
    </div>
  );
}
