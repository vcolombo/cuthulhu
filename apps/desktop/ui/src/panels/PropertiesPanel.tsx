// SPDX-License-Identifier: GPL-3.0-or-later
import type { MachineProfile } from "../App";
import type { Bounds } from "../render/hittest";
import type { LengthUnit } from "../length";
import { displayLength, unitToMm } from "../length";
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
  /** World-space degrees of the one selected shape, or null when rotation does not apply. */
  rotation: number | null;
  onChangeRotation: (degrees: number) => void;
  /** The length unit fields show. Storage stays in millimetres. */
  unit: LengthUnit;
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
  /** A selected shape whose vinyl alpha is under 0.1. It still cuts. */
  faintVinyl?: boolean;
  /** Shown when nothing is selected. Null until a document is loaded. */
  documentInfo: { matLabel: string; passCount: number; objectCount: number } | null;
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

/** Nothing selected: no box, no cuttability, no material. A selected group still has the last two. */
export function showDocumentSection(
  bounds: Bounds | null,
  cutLineType: CutLineTypeJson | "mixed" | null,
  materialPreset: PresetAssignmentJson | "mixed" | undefined,
): boolean {
  return bounds === null && cutLineType === null && materialPreset === undefined;
}

function DesignFor({
  machines,
  machineId,
  onDesignFor,
}: {
  machines: MachineProfile[];
  machineId: string | null;
  onDesignFor: (machineId: string) => void;
}) {
  return (
    <label className="flex flex-col gap-1 text-xs">
      Design for
      <select
        aria-label="Design for"
        value={machineId ?? ""}
        onChange={(e) => {
          if (e.target.value) onDesignFor(e.target.value);
        }}
        className="h-8 rounded-md border border-input bg-background px-2 text-foreground"
      >
        <option value="" disabled>
          Select machine
        </option>
        {machines.map((machine) => (
          <option key={machine.id} value={machine.id}>
            {machine.name}
          </option>
        ))}
      </select>
    </label>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <span className="tabular text-foreground">{value}</span>
    </div>
  );
}

export function PropertiesPanel({
  bounds, cutLineType, materialPreset, effectiveMaterial, presets,
  onChangeX, onChangeY, onChangeW, onChangeH, rotation, onChangeRotation, unit,
  unitCount, editsLocked, distributeBlocked, onAlign, onDistribute,
  onChangeCutLineType, onChangeMaterialPreset, machines, machineId, onDesignFor, faintVinyl,
  documentInfo,
}: Props) {
  const documentSection = showDocumentSection(bounds, cutLineType, materialPreset);
  const step = unit === "in" ? 0.01 : 1;
  const commit = (shown: number) => unitToMm(shown, unit);
  const design = onDesignFor ? (
    <DesignFor machines={machines ?? []} machineId={machineId ?? null} onDesignFor={onDesignFor} />
  ) : null;

  return (
    <div className="flex flex-col gap-2 overflow-y-auto p-2">
      {documentSection && documentInfo ? (
        <section aria-label="Document" className="flex flex-col gap-2">
          <h2 className="text-2xs uppercase text-muted-foreground">Document</h2>
          {design}
          <Fact label="Mat" value={documentInfo.matLabel} />
          <Fact label="Units" value={unit} />
          <Fact label="Passes" value={String(documentInfo.passCount)} />
          <Fact label="Objects" value={String(documentInfo.objectCount)} />
        </section>
      ) : (
        <>
          {design}
          <div className="text-2xs uppercase text-muted-foreground">Properties</div>
          {bounds ? (
            <>
              <NumberField label="X" value={displayLength(bounds.x, unit)} step={step} disabled={editsLocked !== null} onChange={(v) => onChangeX(commit(v))} />
              <NumberField label="Y" value={displayLength(bounds.y, unit)} step={step} disabled={editsLocked !== null} onChange={(v) => onChangeY(commit(v))} />
              <NumberField label="W" value={displayLength(bounds.w, unit)} step={step} min={0} disabled={editsLocked !== null} onChange={(v) => onChangeW(commit(v))} />
              <NumberField label="H" value={displayLength(bounds.h, unit)} step={step} min={0} disabled={editsLocked !== null} onChange={(v) => onChangeH(commit(v))} />
            </>
          ) : null}
          {rotation !== null ? (
            <NumberField label="Rotation" value={rotation} step={1} disabled={editsLocked !== null} onChange={onChangeRotation} />
          ) : null}
          {/* Also outside the bounds branch, since aligning is mostly for multi-node selections. Always shown, disabled
              when it cannot act, so the buttons do not jump the rows below them in and out. */}
          <AlignRow unitCount={unitCount} locked={editsLocked} distributeBlocked={distributeBlocked} onAlign={onAlign} onDistribute={onDistribute} />
          {/* Outside the `bounds` branch: `selectedBounds` is null for every multi-node selection
              and for a selected container (App.tsx), both of which do have a cuttability. */}
          {cutLineType !== null ? (
            <label className="flex items-center gap-1.5 text-xs">
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
          {faintVinyl ? (
            <p className="text-2xs text-muted-foreground">Nearly invisible in the file. It still cuts.</p>
          ) : null}
          {materialPreset !== undefined ? (
            <label className="flex items-center gap-1.5 text-xs">
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
                className="h-8 rounded-md border border-input bg-background px-1 text-foreground"
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
              <span className="text-muted-foreground">
                {materialLabel(materialPreset, effectiveMaterial, presets)}
              </span>
            </label>
          ) : null}
        </>
      )}
    </div>
  );
}
