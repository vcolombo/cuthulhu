// SPDX-License-Identifier: GPL-3.0-or-later
import type { CutStatus, DeviceError, Grouping, PassKey, Phase } from "../../ipc";
import { vinylName } from "../../render/vinyl";
import { parsePassKey, passRowLabel, type PresetLookup } from "../viewmodel";

/** The passes that were actually sent, in cut order. Disabled rows are absent: the worker's
 *  pass index counts enabled passes only, and a disabled colour would name the wrong vinyl. */
export type JobSnapshot = {
  grouping: Grouping;
  passes: { key: PassKey; shapeCount: number }[];
};

export type JobSurface =
  | "hidden"
  | "progress"
  | "swap"
  | "confirm"
  | "failed"
  | "completed"
  | "cancelled"
  | "cancelling";

const UNREAD_PRESETS: PresetLookup = { presets: [], loaded: false };

/** Mirrors `CutStatus::is_active`. A finished cut rests on Idle, so Idle is not active. */
export function jobIsActive(phase: Phase): boolean {
  return phase === "Sending" || phase === "AwaitingConfirmation" || phase === "AwaitingColorSwap" || phase === "Cancelling";
}

/**
 * Which job surface is on screen. Actions win over the phase: a resume is the swap bar even
 * when a caller would have guessed from the phase name, and a confirm with no resume is the
 * panel. `tracking` is this session's send — a status left over from a previous connection
 * does not open the panel on its own.
 */
export function jobSurface(status: CutStatus, tracking: boolean): JobSurface {
  if (!tracking) return "hidden";
  if (status.actions.resume) return "swap";
  if (status.actions.confirm) return "confirm";
  if (status.phase === "Failed") return "failed";
  if (status.phase === "Cancelling") return "cancelling";
  if (status.actions.cancel || status.phase === "Sending") return "progress";
  if (status.ended === "Completed") return "completed";
  if (status.ended === "Cancelled") return "cancelled";
  return "hidden";
}

/** Null when the worker has not reported a total. A zero total is not "0%". */
export function passPercent(sent: { sent: number; total: number } | null): number | null {
  if (sent === null || sent.total <= 0) return null;
  return Math.round((sent.sent / sent.total) * 100);
}

export function percentLabel(pct: number): string {
  return `${pct}% of this pass`;
}

export function loadPrompt(name: string): string {
  return `Load ${name}, then press Resume`;
}

export function waitingLine(
  cutter: string,
  pass: { index: number; total: number } | null,
  shapes: number | null,
): string {
  const parts = [`${cutter} is waiting`];
  if (pass !== null) parts.push(`pass ${pass.index + 1} of ${pass.total}`);
  if (shapes !== null) parts.push(`${shapes} ${shapes === 1 ? "shape" : "shapes"}`);
  return parts.join(" · ");
}

export function lostLabel(seconds: number): string {
  return `Connection lost · last update ${seconds}s ago`;
}

export function lostBanner(name: string): string {
  return `Lost connection to ${name}, the cut is continuing on the host.`;
}

/** Offline cancel stays available, and says it will not land until the host answers. */
export function cancelLabel(place: "panel" | "swap", offline: boolean): string {
  if (offline) return "Cancel (when reconnected)";
  return place === "swap" ? "Cancel job" : "Cancel";
}

/** Plain words for the phase, and the raw error for Details. A missing error still failed. */
export function failureCopy(error: DeviceError | null): { summary: string; details: string } {
  if (error === null) return { summary: "The cut failed.", details: "" };
  if (typeof error === "object") return { summary: "The cutter could not be written to.", details: error.Io };
  switch (error) {
    case "Disconnected":
      return { summary: "The cutter disconnected.", details: error };
    case "Busy":
      return { summary: "The cutter is busy.", details: error };
    case "Timeout":
      return { summary: "The cutter did not answer in time.", details: error };
    case "WriteZero":
      return { summary: "The cutter stopped accepting the job.", details: error };
    default: {
      const never: never = error;
      return { summary: "The cut failed.", details: String(never) };
    }
  }
}

export function placeOf(
  device: { machine_id: string; host: string | null } | null,
  machines: { id: string; name: string }[],
  hosts: { id: string; name: string }[],
): { cutter: string; where: string } {
  if (device === null) return { cutter: "No cutter", where: "" };
  const cutter = machines.find((machine) => machine.id === device.machine_id)?.name ?? device.machine_id;
  const where = device.host === null
    ? "This computer"
    : (hosts.find((host) => host.id === device.host)?.name ?? device.host);
  return { cutter, where };
}

/** The enabled pass the worker is on, named the same way a Send-sheet row is. */
export function currentPass(snapshot: JobSnapshot | null, status: CutStatus): {
  key: PassKey;
  shapeCount: number;
  swatch: string | null;
  name: string;
} | null {
  if (snapshot === null || status.pass === null) return null;
  const pass = snapshot.passes[status.pass.index];
  if (pass === undefined) return null;
  const parsed = parsePassKey(pass.key);
  if (parsed.kind === "color" && parsed.color !== null) {
    const swatch = `#${(parsed.color >>> 8).toString(16).padStart(6, "0")}`;
    return { key: pass.key, shapeCount: pass.shapeCount, swatch, name: vinylName(swatch) };
  }
  const label = passRowLabel(pass.key, UNREAD_PRESETS, snapshot.grouping);
  return { key: pass.key, shapeCount: pass.shapeCount, swatch: label.swatch, name: label.text ?? pass.key };
}
