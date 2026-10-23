// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useState, type ReactNode } from "react";
import { Crosshair, Maximize2, ZoomIn } from "lucide-react";
import type { MachineProfile } from "../App";
import type { LengthUnit } from "../length";
import { formatLength } from "../length";
import type { CutStatus, DeviceInfo, PairedHostView, Phase } from "../ipc";
import { committedRollLength, matKindLabel, type MatKind } from "../render/mat";

type MatControl = {
  kind: MatKind;
  kinds: MatKind[];
  rollLengthMm: number;
  onKind: (kind: MatKind) => void;
  onRollLength: (mm: number) => void;
};

type Props = {
  device: DeviceInfo | null;
  profile: MachineProfile | null;
  machines: MachineProfile[];
  hosts: PairedHostView[];
  /** Null until a document is loaded. The mat is view-only; the bed is not what this bar reports. */
  mat: MatControl | null;
  status: CutStatus;
  /** Null until a document is loaded: there is no mat to be zoomed relative to. */
  zoomPercent: number | null;
  /** Pointer position in document millimetres, or null when the pointer is off the canvas. */
  cursor: { x: number; y: number } | null;
  /** Selection bounds in document millimetres, or null when nothing is selected. */
  selection: { w: number; h: number } | null;
  unit: LengthUnit;
  /** The folded colour-swap chip. It replaces the mini progress, which would say the same thing twice. */
  trailing?: ReactNode;
  /** The job panel or the swap bar already shows this job. */
  hideJobEcho?: boolean;
};

function phaseDot(phase: Phase): string {
  if (phase === "Idle" || phase === "Disconnected") return "bg-success";
  if (phase === "Failed") return "bg-destructive";
  return "bg-accent-ink";
}

function RollLength({ mm, unit, onCommit }: { mm: number; unit: LengthUnit; onCommit: (mm: number) => void }) {
  const [text, setText] = useState(() => formatLength(mm, unit));
  useEffect(() => setText(formatLength(mm, unit)), [mm, unit]);
  const commit = () => {
    const next = committedRollLength(text, unit);
    if (next === null) {
      setText(formatLength(mm, unit));
      return;
    }
    // Write the clamped length back before the parent hears it. A value already at the
    // minimum does not change `mm`, so the effect would leave a typed "2" showing a 6 in roll.
    setText(formatLength(next, unit));
    onCommit(next);
  };
  return (
    <input
      aria-label="Roll length"
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
      }}
      className="h-6 w-16 rounded-sm border border-border bg-panel px-1 text-xs text-foreground"
    />
  );
}

/** The cursor string the wheel-pan test reads. Millimetres keep two spaces before `y`. */
function cursorText(cursor: { x: number; y: number }, unit: LengthUnit): string {
  if (unit === "mm") return `x ${cursor.x.toFixed(1)}  y ${cursor.y.toFixed(1)} mm`;
  return `x ${formatLength(cursor.x, "in")}  y ${formatLength(cursor.y, "in")} in`;
}

export function StatusBar({ device, profile, machines, hosts, mat, status, zoomPercent, cursor, selection, unit, trailing = null, hideJobEcho = false }: Props) {
  const connectedName = device ? (machines.find((machine) => machine.id === device.machine_id)?.name ?? device.machine_id) : null;
  const where = device
    ? device.host === null
      ? "USB · this computer"
      : (hosts.find((host) => host.id === device.host)?.name ?? device.host)
    : null;
  const machineText = device && connectedName
    ? `${connectedName} · ${where}`
    : profile
      ? `Designing for ${profile.name}`
      : "No cutter";
  const busy = status.phase === "Sending" || status.phase === "AwaitingColorSwap" || status.phase === "AwaitingConfirmation";
  const progress = status.sent && status.sent.total > 0 ? Math.round((status.sent.sent / status.sent.total) * 100) : null;

  return (
    <footer className="flex h-8 shrink-0 items-center gap-4 overflow-hidden whitespace-nowrap border-t border-border bg-panel px-3 text-xs text-muted-foreground">
      <span className={`inline-block size-2 shrink-0 rounded-full ${phaseDot(status.phase)}`} aria-hidden />
      <span className="truncate text-foreground">{machineText}</span>
      {mat ? (
        <span className="flex items-center gap-1.5">
          <span>Mat</span>
          <select
            aria-label="Mat size"
            value={mat.kind}
            onChange={(e) => mat.onKind(e.target.value as MatKind)}
            className="h-6 rounded-sm border border-border bg-panel px-1 text-xs text-foreground"
          >
            {mat.kinds.map((kind) => (
              <option key={kind} value={kind}>
                {matKindLabel(kind, unit)}
              </option>
            ))}
          </select>
          {mat.kind === "roll" ? (
            <RollLength mm={mat.rollLengthMm} unit={unit} onCommit={mat.onRollLength} />
          ) : null}
        </span>
      ) : null}
      <span className="flex items-center gap-1.5">
        <Crosshair className="size-3.5" aria-hidden />
        {cursor ? (
          <span data-testid="status-cursor" className="tabular">
            {cursorText(cursor, unit)}
          </span>
        ) : (
          <span>—</span>
        )}
      </span>
      <span className="flex items-center gap-1.5">
        <Maximize2 className="size-3.5" aria-hidden />
        {selection ? (
          <span>
            {formatLength(selection.w, unit)} × {formatLength(selection.h, unit)} {unit}
          </span>
        ) : (
          <span>No selection</span>
        )}
      </span>
      {zoomPercent !== null ? (
        <span className="flex items-center gap-1.5">
          <ZoomIn className="size-3.5" aria-hidden />
          <span data-testid="status-zoom" className="tabular">
            {zoomPercent}%
          </span>
        </span>
      ) : null}
      <div className="flex-1" />
      {trailing}
      {trailing === null && busy && !hideJobEcho ? (
        <span className="flex items-center gap-2 text-foreground">
          <span>
            {status.pass ? `pass ${status.pass.index + 1}/${status.pass.total}` : status.phase}
          </span>
          {progress !== null ? (
            <span className="h-1 w-16 overflow-hidden rounded-full bg-subtle">
              <span className="block h-full bg-success" style={{ width: `${progress}%` }} />
            </span>
          ) : null}
        </span>
      ) : null}
    </footer>
  );
}

