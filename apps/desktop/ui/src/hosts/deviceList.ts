// SPDX-License-Identifier: GPL-3.0-or-later
import { ipcErrorMessage, type CutStatus, type DeviceInfo, type PairedHostView } from "../ipc";

export type DeviceSection = {
  /** null for cutters attached to this computer. */
  hostId: string | null;
  title: string;
  address: string | null;
  /** Why this section is not normal — unreachable, or naming a host nobody is paired with. */
  unreachable: string | null;
  devices: DeviceInfo[];
  /** These are the last values a poll managed to read, not what is true now. */
  stale: boolean;
};

// An unreachable host keeps its section and its cutters, with the reason shown, rather than
// being filtered out — a cutter that vanishes looks exactly like one that was never paired
// (#42). A paired host with no cutters still gets a section: "nothing attached" and "does not
// exist" are different facts.
export function groupDevices(
  devices: DeviceInfo[], hosts: PairedHostView[],
): DeviceSection[] {
  const local: DeviceSection = {
    hostId: null, title: "This computer", address: null, unreachable: null, stale: false,
    devices: devices.filter(d => d.host === null),
  };
  const known = hosts.map(h => ({
    hostId: h.id, title: h.name, address: h.address, unreachable: h.unreachable, stale: false,
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
    hostId: id, title: id, address: null, stale: false,
    unreachable: "this Cut Host is not paired with this computer",
    devices: devices.filter(d => d.host === id),
  }))];
}

/** The host list after a forget, and the refusal to show if it did not happen. */
export type ForgetResult = { hosts: PairedHostView[]; message: string | null };

/**
 * Forget a Cut Host, all or nothing.
 *
 * The row goes only once the Rust side has agreed, never optimistically: it refuses with
 * `host_busy` while a cut is active on that host, because discarding the token would strand a Job
 * the desktop can no longer cancel — a blade still moving on the Pi with nothing left to stop it.
 * Removing the row first and restoring it on refusal would show that host as gone, however
 * briefly, at the one moment it most needs to be reachable.
 *
 * A refusal leaves the list untouched rather than partly erased. Nothing here clears a stored
 * token on its own: re-pairing replaces it, and a host that plainly needs attention is better
 * than one whose credentials this side quietly dropped.
 *
 * The refusal is the Rust side's own prose, unaltered (#94).
 */
export async function forgetFrom(
  hosts: PairedHostView[], id: string, forget: (id: string) => Promise<void>,
): Promise<ForgetResult> {
  try {
    await forget(id);
    return { hosts: hosts.filter(h => h.id !== id), message: null };
  } catch (e) {
    return { hosts, message: ipcErrorMessage(e) };
  }
}

/**
 * What to show for a host whose poll failed: the last section that worked, marked stale.
 *
 * Not an empty section and not a reset one. The cut is still running on the Pi — the poll failing
 * says something about the network, not about the job — and a row that blanks or drops back to
 * idle reads as "finished", which is the one wrong thing to tell someone whose material is still
 * moving under a blade. The cutters stay listed with whatever status each already had.
 */
export function staleSection(previous: DeviceSection, e: unknown): DeviceSection {
  return { ...previous, stale: true, unreachable: ipcErrorMessage(e) };
}

// A caller is told about a cut through one value, `CutStatus.actions` — never `phase`. Two
// cutters can share the same phase (e.g. "Disconnected") while one is legal to cut and the
// other isn't; only `actions` tells them apart. `phase` may still end up in UI copy as
// descriptive text, but nothing below branches on it.
export function deviceBadge(status: CutStatus | null): { label: string; tone: "idle" | "busy" | "attention" | "unknown" | "gone" } {
  if (status === null) {
    // No status has arrived for this cutter yet (distinct from DISCONNECTED_STATUS, which is a
    // known "nothing legal" state). Not "gone" — we haven't tried and failed, we just don't know
    // — and not "idle" either, since that would offer a cut before one is confirmed legal.
    //
    // Its own tone, not "attention", which conflated two different things: "we may not offer a
    // cut" and "the operator should look at this". Only the first is true here, and every cutter
    // on a freshly opened dialog is in this state — a UI that raises an alarm about the most
    // ordinary thing it ever shows has spent the alarm before anything is wrong.
    return { label: "Unknown", tone: "unknown" };
  }
  const { actions, ended } = status;
  if (actions.cut) {
    return { label: ended === "Cancelled" ? "Ready (last cut cancelled)" : "Ready", tone: "idle" };
  }
  // The one tone that asks for a person: a Puma parked mid-Job waiting for a colour swap is
  // someone being asked to walk over to it.
  if (actions.confirm || actions.resume) {
    return { label: "Needs attention", tone: "attention" };
  }
  if (actions.cancel) {
    return { label: "Cutting", tone: "busy" };
  }
  // A cancel with nothing legal left: the Job ended, but no poll saw the machine come to rest
  // — the ordinary outcome on a Puma, whose abort is queued behind whatever motion is already
  // buffered. Still derived from `actions` and `ended`, not from the phase (which is `Idle`
  // here, the same as a confirmed stop). This is why the operator is asked to look: only they
  // can tell, and reconnecting the cutter is what says they have.
  if (ended === "Cancelled") {
    return { label: "Cancelled — stop not confirmed, check the cutter", tone: "attention" };
  }
  // Nothing is legal. A reachable, idle cutter always has `actions.cut`, so this combination
  // means the cutter (or its host) can't be reached right now — same fact `groupDevices` keeps
  // visible rather than hiding (#42).
  return { label: "Unreachable", tone: "gone" };
}

/**
 * Whether the connected cutter's row offers a disconnect, and what it should say.
 *
 * It has to offer one at all because a stop nothing confirmed refuses another cut until the
 * transport is re-opened (`driver-core`'s `Cancelled` arm), and `DeviceManager::connect` refuses
 * from that state too — so with no way to disconnect, one cancelled Puma ends the session.
 *
 * `null` while a Job is in flight: a disconnect there drops the transport under a moving blade
 * and abandons the Job silently. "A Job exists" is read from `actions` — the only three verbs a
 * Job ever offers — not from the phase.
 */
export function connectedControl(status: CutStatus | null): string | null {
  if (status !== null && (status.actions.cancel || status.actions.resume || status.actions.confirm)) {
    return null;
  }
  return "Disconnect";
}
