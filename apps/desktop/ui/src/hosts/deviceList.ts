// SPDX-License-Identifier: GPL-3.0-or-later
import type { DeviceInfo, PairedHostView } from "../ipc";

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
  return [local, ...hosts.map(h => ({
    hostId: h.id, title: h.name, address: h.address, unreachable: h.unreachable,
    devices: devices.filter(d => d.host === h.id),
  }))];
}
