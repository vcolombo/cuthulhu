// SPDX-License-Identifier: GPL-3.0-or-later
import type { MachineProfile } from "../App";
import type { CutStatus, Phase } from "../ipc";

type Props = {
  machine: MachineProfile | null;
  artboard: { w: number; h: number } | null;
  error: string | null;
  status: CutStatus;
  /** Null until a document is loaded: there is no artboard to be zoomed relative to. */
  zoomPercent: number | null;
  /** Pointer position in document mm, or null when the pointer is off the canvas. */
  cursor: { x: number; y: number } | null;
};

// Idle/Disconnected read as "nothing wrong" (green), a failed device is red, and every
// other phase (connecting, actively cutting, cancelling…) is "busy" (accent).
function dotColor(phase: Phase): string {
  if (phase === "Idle" || phase === "Disconnected") return "var(--ready)";
  if (phase === "Failed") return "var(--cut)";
  return "var(--accent-ink)";
}

export function StatusBar({ machine, artboard, error, status, zoomPercent, cursor }: Props) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "4px 10px",
        fontSize: 11,
        color: "var(--muted-foreground)",
        background: "var(--panel)",
        borderTop: "1px solid var(--border)",
      }}
    >
      <span style={{ width: 8, height: 8, borderRadius: "50%", background: dotColor(status.phase), display: "inline-block" }} />
      <span>{machine ? machine.name : "No machine selected"}</span>
      {artboard ? (
        <span>
          {artboard.w} x {artboard.h} mm
        </span>
      ) : null}
      <div style={{ flex: 1 }} />
      {cursor ? (
        <span data-testid="status-cursor" style={{ fontVariantNumeric: "tabular-nums" }}>
          x {cursor.x.toFixed(1)}  y {cursor.y.toFixed(1)} mm
        </span>
      ) : null}
      {zoomPercent !== null ? (
        <span data-testid="status-zoom" style={{ fontVariantNumeric: "tabular-nums" }}>
          {zoomPercent}%
        </span>
      ) : null}
      {error ? <span style={{ color: "var(--cut)" }}>{error}</span> : null}
    </div>
  );
}
