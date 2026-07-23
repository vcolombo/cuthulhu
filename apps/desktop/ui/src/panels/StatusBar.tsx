// SPDX-License-Identifier: GPL-3.0-or-later
import type { MachineProfile } from "../App";

type Props = {
  machine: MachineProfile | null;
  artboard: { w: number; h: number } | null;
  error: string | null;
};

export function StatusBar({ machine, artboard, error }: Props) {
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
      <span style={{ width: 8, height: 8, borderRadius: "50%", background: "var(--ready)", display: "inline-block" }} />
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
