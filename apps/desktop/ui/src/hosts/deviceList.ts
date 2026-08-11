// SPDX-License-Identifier: GPL-3.0-or-later
import type { CutStatus, DeviceInfo, PairedHostView } from "../ipc";

export type DeviceSection = {
  /** null for cutters attached to this computer. */
  hostId: string | null;
  title: string;
  address: string | null;
  unreachable: string | null;
  devices: DeviceInfo[];
};

// An unreachable host keeps its section and its cutters, with the reason shown, rather than
// being filtered out — a cutter that vanishes looks exactly like one that was never paired
// (#42). A paired host with no cutters still gets a section: "nothing attached" and "does not
// exist" are different facts.
export function groupDevices(
  devices: DeviceInfo[], hosts: PairedHostView[],
): DeviceSection[] {
  const local: DeviceSection = {
    hostId: null, title: "This computer", address: null, unreachable: null,
    devices: devices.filter(d => d.host === null),
  };
  const known = hosts.map(h => ({
    hostId: h.id, title: h.name, address: h.address, unreachable: h.unreachable,
    devices: devices.filter(d => d.host === h.id),
  }));
  // A cutter whose host is in neither list matches no section above — not local, since its `host`
  // is set, and not any paired host's. Dropping it here would be the same disappearance the rest
  // of this function exists to prevent, and it is reachable without anything being wrong: a poll
  // whose `list_devices` resolved before a forget and whose `list_hosts` resolved after sees
  // exactly this. It gets its own section rather than a guess at which host it meant.
  const paired = new Set(hosts.map(h => h.id));
  const orphaned = [...new Set(
    devices.flatMap(d => (d.host !== null && !paired.has(d.host) ? [d.host] : [])),
  )];
  return [local, ...known, ...orphaned.map(id => ({
    hostId: id, title: id, address: null,
    unreachable: "this Cut Host is not paired with this computer",
    devices: devices.filter(d => d.host === id),
  }))];
}

// A caller is told about a cut through one value, `CutStatus.actions` — never `phase`. Two
// cutters can share the same phase (e.g. "Disconnected") while one is legal to cut and the
// other isn't; only `actions` tells them apart. `phase` may still end up in UI copy as
// descriptive text, but nothing below branches on it.
export function deviceBadge(status: CutStatus | null): { label: string; tone: "idle" | "busy" | "attention" | "gone" } {
  if (status === null) {
    // No status has arrived for this cutter yet (distinct from DISCONNECTED_STATUS, which is a
    // known "nothing legal" state). Not "gone" — we haven't tried and failed, we just don't know
    // — and not "idle" either, since that would offer a cut before one is confirmed legal.
    return { label: "Unknown", tone: "attention" };
  }
  const { actions, ended } = status;
  if (actions.cut) {
    return { label: ended === "Cancelled" ? "Ready (last cut cancelled)" : "Ready", tone: "idle" };
  }
  if (actions.confirm || actions.resume) {
    return { label: "Needs attention", tone: "attention" };
  }
  if (actions.cancel) {
    return { label: "Cutting", tone: "busy" };
  }
  // Nothing is legal. A reachable, idle cutter always has `actions.cut`, so this combination
  // means the cutter (or its host) can't be reached right now — same fact `groupDevices` keeps
  // visible rather than hiding (#42).
  return { label: "Unreachable", tone: "gone" };
}
