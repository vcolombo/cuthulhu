// SPDX-License-Identifier: GPL-3.0-or-later
import type { CutStatus, DeviceError, Grouping, PassKey, Phase } from "../../ipc";

/** Every pass row from the accepted cut, in plan order, including rows that were switched off.
 *  The worker's pass index counts only the passes it was given (`PassPosition` into that vec,
 *  `driver-core` `status.rs`). A disabled row left in the middle would name the wrong vinyl if
 *  the index were used on this list directly, so `currentPass` skips those rows. */
export type JobSnapshot = {
  grouping: Grouping;
  /** Name and swatch are resolved when the cut is accepted. Reading them again later would
   *  see an empty preset list and call every preset pass "(name unread)" for the whole job. */
  passes: { key: PassKey; shapeCount: number; enabled: boolean; name: string; swatch: string | null }[];
  /** The id this accept was given. A remote accept reports 0, and the status's own id is
   *  what a later poll is matched against. */
  jobId: number;
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

/** A name standing in a sentence. A hex has no case to change; "Gold vinyl" becomes "gold vinyl". */
export function vinylInSentence(name: string): string {
  if (name.startsWith("#")) return name;
  return name.length === 0 ? name : name.charAt(0).toLowerCase() + name.slice(1);
}

export function loadPrompt(name: string): string {
  return `Load ${vinylInSentence(name)}, then press Resume`;
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

/** The address is its own token. The banner breaks inside host:port and on word boundaries elsewhere.
 *  A truncated host is the one an operator cannot redial, so the token is the whole address. */
export function reconnectingCopy(name: string, address: string): { before: string; address: string | null; after: string } {
  if (address === "") return { before: `Reconnecting to ${name}. The cut keeps going on the host.`, address: null, after: "" };
  return { before: `Reconnecting to ${name} (`, address, after: "). The cut keeps going on the host." };
}

export function reconnectingLine(name: string, address: string): string {
  const copy = reconnectingCopy(name, address);
  return copy.address === null ? copy.before : `${copy.before}${copy.address}${copy.after}`;
}

/** Stripes mean the blade is still taking bytes. A full pass, a confirm, and a frozen host are still. */
export function passLive(phase: Phase, pct: number | null, offline: boolean): boolean {
  return !offline && phase === "Sending" && pct !== null && pct < 100;
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

/** The enabled pass the worker is on. `status.pass.index` counts submitted passes, so a row
 *  that was not sent is skipped rather than loaded. */
export function currentPass(snapshot: JobSnapshot | null, status: CutStatus): {
  key: PassKey;
  shapeCount: number;
  swatch: string | null;
  name: string;
} | null {
  if (snapshot === null || status.pass === null) return null;
  const pass = snapshot.passes.filter((row) => row.enabled)[status.pass.index];
  if (pass === undefined) return null;
  return { key: pass.key, shapeCount: pass.shapeCount, swatch: pass.swatch, name: pass.name };
}
