// SPDX-License-Identifier: GPL-3.0-or-later
import type { MachineProfile } from "../App";
import type { DeviceState } from "../ipc";

type Props = {
  machine: MachineProfile | null;
  artboard: { w: number; h: number } | null;
  error: string | null;
  deviceState: DeviceState;
};

// Idle/Disconnected read as "nothing wrong" (green), a device error is red, and every
// other state (connecting, actively cutting, cancelling…) is "busy" (accent).
function dotColor(state: DeviceState): string {
  if (state === "Idle" || state === "Disconnected") return "var(--ready)";
  if (typeof state === "object" && "Error" in state) return "var(--cut)";
  return "var(--accent)";
}

export function StatusBar({ machine, artboard, error, deviceState }: Props) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "4px 10px",
        fontSize: 11,
        color: "var(--muted)",
        background: "var(--panel)",
        borderTop: "1px solid var(--border)",
      }}
    >
      <span style={{ width: 8, height: 8, borderRadius: "50%", background: dotColor(deviceState), display: "inline-block" }} />
      <span>{machine ? machine.name : "No machine selected"}</span>
      {artboard ? (
        <span>
          {artboard.w} x {artboard.h} mm
        </span>
      ) : null}
      <div style={{ flex: 1 }} />
      {error ? <span style={{ color: "var(--cut)" }}>{error}</span> : null}
    </div>
  );
}
