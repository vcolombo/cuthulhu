// SPDX-License-Identifier: GPL-3.0-or-later
import type { ReactNode } from "react";
import type { Scene } from "../../render/hittest";
import { vinylDisplay } from "../../render/vinyl";
import type { ResolvedTheme } from "../../theme";
import type { MachineProfile } from "../../App";
import type { DeviceInfo, Grouping, PairedHostView } from "../../ipc";
import { CutPreview } from "../CutPreview";
import { effectiveSettings, passRowLabel, type PresetLookup } from "../viewmodel";
import type { PassRow } from "./useCutPlan";

/** The host had already accepted this dispatch. */
export function AlreadyAcceptedNote({ show }: { show: boolean }) {
  if (!show) return null;
  return (
    <div role="alert" className="text-sm text-destructive">
      This Cut Host had already accepted this cut, so nothing new was started — this was read
      as a retry of a dispatch whose answer was lost. If the cutter is idle and you meant a
      fresh sheet, press Cut again.
    </div>
  );
}

export function StepPreview({
  scene,
  artboard,
  passes,
  travel,
  theme,
  connected,
  hosts,
  machines,
  presetLookup,
  grouping,
  weedOn,
}: {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  passes: PassRow[];
  travel: [number, number, number, number][];
  theme: ResolvedTheme;
  connected: DeviceInfo | null;
  hosts: PairedHostView[];
  machines: MachineProfile[];
  presetLookup: PresetLookup;
  grouping: Grouping;
  weedOn: boolean;
}): ReactNode {
  const cutting = passes.filter((pass) => pass.enabled);
  const host = connected?.host == null ? null : hosts.find((item) => item.id === connected.host) ?? null;
  const runsOn = host ? host.name : "This computer";
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_220px] items-start gap-4">
      <div className="rounded-lg border border-border bg-canvas p-3">
        <CutPreview scene={scene} artboard={artboard} passes={passes} travel={travel} theme={theme} />
        <p className="mt-2 text-xs text-muted-foreground">Numbers show the cut order. Each colour is a separate pass.</p>
      </div>
      <div className="min-w-0 space-y-3 text-sm">
        <dl className="space-y-1.5 rounded-lg border border-border bg-card p-3 text-xs">
          <Row label="Cutter" value={connected ? (machines.find((machine) => machine.id === connected.machine_id)?.name ?? connected.machine_id) : "None"} />
          <Row label="Runs on" value={runsOn} />
          <Row label="Material" value={materialName(cutting, presetLookup)} />
          <Row label="Colour passes" value={String(cutting.length)} />
          <Row label="Weed box" value={weedOn ? "On" : "Off"} />
        </dl>
        <div className="space-y-1">
          {cutting.map((pass, index) => {
            const label = passRowLabel(pass.key, presetLookup, grouping);
            const swatch = label.swatch !== null ? vinylDisplay(label.swatch, theme).swatch : null;
            const effective = effectiveSettings(pass, presetLookup);
            return (
              <div key={pass.key} className="flex items-start gap-2 text-xs">
                <span className="tabular text-muted-foreground">{index + 1}.</span>
                {swatch !== null ? <span className="mt-0.5 inline-block size-3 shrink-0 rounded-sm border border-input" style={{ background: swatch }} /> : null}
                {/* min-w-0 and break-words: a name with no space is wider than this column, and the
                    column's min-content would otherwise widen the sheet until the name is clipped. */}
                <span className="min-w-0">
                  <span className="block break-words">{label.text ?? "This pass"}</span>
                  <span className="mt-0.5 block tabular text-muted-foreground">
                    Speed {shown(effective.speed)} · force {shown(effective.force)} · {shown(effective.repeatCount)}×
                  </span>
                </span>
              </div>
            );
          })}
        </div>
        <p className={host ? "break-words rounded-md bg-success/10 p-2 text-xs" : "break-words rounded-md bg-warning/10 p-2 text-xs"}>
          {host
            ? `Runs on ${host.name}. You can close this computer once the cut has started.`
            : "Runs from this computer — keep Cuthulhu open until the cut finishes."}
        </p>
      </div>
    </div>
  );
}

/** The value wraps inside the column. One token wider than the column was the column's
 *  min-content, and the sheet scrolled sideways to show it. */
function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-3">
      <dt className="shrink-0 text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-words text-right">{value}</dd>
    </div>
  );
}

function materialName(passes: PassRow[], lookup: PresetLookup): string {
  const ids = [...new Set(passes.map((pass) => pass.presetId))];
  if (ids.length === 0) return "No preset";
  if (ids.length > 1) return "Mixed";
  const id = ids[0];
  if (id == null) return "No preset";
  return lookup.presets.find((preset) => preset.id === id)?.name ?? id;
}

function shown(value: number | null): string {
  return value === null ? "—" : String(value);
}
