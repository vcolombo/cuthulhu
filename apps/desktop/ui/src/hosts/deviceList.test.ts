// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { connectedControl, deviceBadge, forgetFrom, groupDevices, staleSection } from "./deviceList";
import type { CutStatus, DeviceInfo, PairedHostView } from "../ipc";

const aDevice = (): DeviceInfo => ({
  instance_id: "usb:mock",
  machine_id: "cameo5",
  transport: { Usb: { locator: "mock" } },
  candidate: false,
  host: null,
});

const aStatus = (): CutStatus => ({
  phase: "Disconnected",
  ended: null,
  actions: { cut: false, cancel: false, resume: false, confirm: false },
  pass: null,
  sent: null,
  error: null,
});

describe("groupDevices", () => {
  it("keeps an unreachable host's cutters listed, with the reason", () => {
    const hosts: PairedHostView[] = [{ id: "host-1", name: "Workshop Pi", address: "pi.local:7878",
                                       unreachable: "the host could not be reached (timed out)" }];
    const devices: DeviceInfo[] = [{ ...aDevice(), instance_id: "usb:sn:A", host: "host-1" }];
    const sections = groupDevices(devices, hosts);
    const remote = sections.find(s => s.hostId === "host-1")!;
    expect(remote.devices).toHaveLength(1);
    expect(remote.unreachable).toContain("could not be reached");
  });

  it("lists a paired host that has no cutters, rather than hiding it", () => {
    const sections = groupDevices([], [{ id: "host-1", name: "Workshop Pi",
                                         address: "pi.local:7878", unreachable: null }]);
    expect(sections.find(s => s.hostId === "host-1")).toBeDefined();
  });

  it("puts this computer's cutters first", () => {
    const sections = groupDevices(
      [{ ...aDevice(), host: "host-1" }, { ...aDevice(), instance_id: "usb:sn:L", host: null }],
      [{ id: "host-1", name: "Workshop Pi", address: "pi.local:7878", unreachable: null }],
    );
    expect(sections[0].hostId).toBeNull();
  });

  // Reachable as a race: list_devices resolves before a forget and list_hosts after, so a cutter
  // names a host that is no longer in the list. It is not local (its `host` is not null) and it
  // matches no host's section, so it used to land in none of them and blink out of the list —
  // exactly the disappearance this module exists to prevent (#42).
  it("gives a cutter naming an unpaired host somewhere to go, rather than dropping it", () => {
    const orphan: DeviceInfo = { ...aDevice(), instance_id: "usb:sn:A", host: "host-gone" };
    const sections = groupDevices([orphan], []);
    expect(sections.flatMap(s => s.devices)).toContainEqual(orphan);
    const unknown = sections.find(s => s.hostId === "host-gone")!;
    expect(unknown.unreachable).not.toBeNull();
  });

  it("keeps every orphan, and does not invent a section for a host that has one already", () => {
    const devices: DeviceInfo[] = [
      { ...aDevice(), instance_id: "usb:sn:A", host: "host-gone" },
      { ...aDevice(), instance_id: "usb:sn:B", host: "host-gone" },
      { ...aDevice(), instance_id: "usb:sn:C", host: "host-also-gone" },
      { ...aDevice(), instance_id: "usb:sn:D", host: "host-1" },
    ];
    const sections = groupDevices(devices, [{ id: "host-1", name: "Workshop Pi",
                                              address: "pi.local:7878", unreachable: null }]);
    expect(sections.flatMap(s => s.devices)).toHaveLength(devices.length);
    expect(sections.filter(s => s.hostId === "host-gone")).toHaveLength(1);
    // A known host keeps its own section; only the unpaired ids get made up.
    expect(sections.find(s => s.hostId === "host-1")!.title).toBe("Workshop Pi");
  });
});

const PAIRED: PairedHostView[] = [
  { id: "host-1", name: "Workshop Pi", address: "pi.local:7878", unreachable: null },
  { id: "host-2", name: "Office Pi", address: "office.local:7878", unreachable: null },
];

const runForget = (id: string, forget: (id: string) => Promise<void>) =>
  forgetFrom(PAIRED, id, forget);

describe("forgetFrom", () => {
  it("keeps a host that refuses to be forgotten, and says why", async () => {
    // The Rust side refuses while a cut is active on that host: the desktop would otherwise
    // discard the token for a Job it can no longer cancel.
    const s = await runForget("host-1", async () => {
      throw { code: "host_busy", message: "a cut is active on this host; cancel it before forgetting" };
    });
    expect(s.hosts.map(h => h.id)).toContain("host-1");
    expect(s.message).toContain("cancel it before forgetting");
  });

  it("removes the host only once the Rust side has agreed", async () => {
    const s = await runForget("host-1", async () => {});
    expect(s.hosts.map(h => h.id)).toEqual(["host-2"]);
    expect(s.message).toBeNull();
  });

  // The stored token is the Rust side's, kept until re-pairing replaces it. A refusal must leave
  // the list untouched rather than half-erased: a host that plainly needs attention is better
  // than one whose credentials this side quietly dropped.
  it("leaves the list exactly as it was when the forget is refused", async () => {
    const s = await runForget("host-1", async () => {
      throw { code: "host_busy", message: "a cut is active on this host; cancel it before forgetting" };
    });
    expect(s.hosts).toEqual(PAIRED);
  });

  it("removes nothing when the forget names a host that is not listed", async () => {
    const s = await runForget("host-gone", async () => {});
    expect(s.hosts).toEqual(PAIRED);
  });
});

describe("staleSection", () => {
  const section = () => groupDevices(
    [{ ...aDevice(), instance_id: "usb:sn:A", host: "host-1" }],
    [{ id: "host-1", name: "Workshop Pi", address: "pi.local:7878", unreachable: null }],
  ).find(s => s.hostId === "host-1")!;

  // The cut is still running on the Pi. A row that blanks or resets reads as "finished", which is
  // the one wrong thing to tell someone whose material is still moving under a blade.
  it("keeps the cutters a failed poll could not re-list, and marks them stale", () => {
    const s = staleSection(section(), { code: "host_unreachable", message: "the host could not be reached (timed out)" });
    expect(s.devices).toEqual(section().devices);
    expect(s.stale).toBe(true);
    expect(s.title).toBe("Workshop Pi");
  });

  it("shows the host's own words for why the poll failed", () => {
    const s = staleSection(section(), { code: "host_unreachable", message: "the host could not be reached (timed out)" });
    expect(s.unreachable).toBe("the host could not be reached (timed out)");
  });

  it("clears stale once a poll succeeds, since groupDevices builds the section afresh", () => {
    expect(section().stale).toBe(false);
  });
});

describe("deviceBadge", () => {
  it("decides what is offered from actions, never from the phase", () => {
    // A cutter on a host we cannot reach reports Disconnected with every action false. The badge
    // must say so, and nothing may offer a cut for it.
    const gone: CutStatus = { ...aStatus(), actions: { cut: false, cancel: false, resume: false, confirm: false } };
    expect(deviceBadge(gone).tone).toBe("gone");

    // Same phase string, different actions: a cut is legal. If the badge read `phase` these two
    // would be indistinguishable, which is the bug this guards.
    const ready: CutStatus = { ...gone, actions: { ...gone.actions, cut: true } };
    expect(deviceBadge(ready).tone).toBe("idle");
  });

  // Its own tone, and quiet. Not "gone" (nothing has been tried and failed) and not "idle"
  // (that would offer a cut before one is legal) — but not "attention" either, which is the tone
  // that asks for a person. Every cutter on a freshly opened dialog is unpolled, so an alarm
  // here is an alarm about the most ordinary state the dialog has.
  it("gives an unpolled cutter its own tone, so it does not read as needing a person", () => {
    expect(deviceBadge(null).tone).toBe("unknown");
  });

  it("keeps attention for the states that genuinely want the operator", () => {
    const swap: CutStatus = { ...aStatus(), actions: { ...aStatus().actions, resume: true } };
    expect(deviceBadge(swap).tone).toBe("attention");
    expect(deviceBadge(null).tone).not.toBe("attention");
  });

  it("flags AwaitingConfirmation-style states even though their phase differs from Idle", () => {
    const status: CutStatus = { ...aStatus(), phase: "AwaitingColorSwap", actions: { ...aStatus().actions, confirm: true } };
    expect(deviceBadge(status).tone).toBe("attention");
  });

  it("shows a running job as busy when only cancel is legal", () => {
    const status: CutStatus = { ...aStatus(), phase: "Sending", actions: { ...aStatus().actions, cancel: true } };
    expect(deviceBadge(status).tone).toBe("busy");
  });

  // Both rest on "Idle" and both report a cancelled ending, so only `actions.cut` separates a
  // stop the machine confirmed from one nothing witnessed. The second must not read as ready,
  // and must not read as "Unreachable" either — the cutter is right there, and someone needs
  // to look at it.
  it("tells a confirmed stop from one nothing saw, and asks for a person on the second", () => {
    const confirmed: CutStatus = { ...aStatus(), phase: "Idle", ended: "Cancelled",
                                   actions: { ...aStatus().actions, cut: true } };
    expect(deviceBadge(confirmed).tone).toBe("idle");

    const unconfirmed: CutStatus = { ...confirmed, actions: { ...aStatus().actions, cut: false } };
    expect(deviceBadge(unconfirmed).tone).toBe("attention");
    expect(deviceBadge(unconfirmed).label).toMatch(/not confirmed/);
  });
});

describe("connectedControl", () => {
  // The state this exists for. Both a cut and a connect are refused there, so a row with no
  // disconnect leaves the operator restarting the app to use the cutter again.
  it("offers a disconnect after a cancel whose stop was never confirmed", () => {
    const unconfirmed: CutStatus = { ...aStatus(), phase: "Idle", ended: "Cancelled" };
    expect(connectedControl(unconfirmed, false)).toEqual({ label: "Disconnect", verb: "disconnect" });
    expect(connectedControl(null, false)).toEqual({ label: "Disconnect", verb: "disconnect" });
  });

  // A remote cutter's transport belongs to the Pi, so this desktop's disconnect only drops the
  // aim and leaves the cutter as stuck as it was. Only the host's own verb re-opens it.
  it("asks a Cut Host to re-open its own cutter rather than dropping the aim", () => {
    const unconfirmed: CutStatus = { ...aStatus(), phase: "Idle", ended: "Cancelled" };
    expect(connectedControl(unconfirmed, true)).toEqual({ label: "Reconnect", verb: "reconnect" });
  });

  // Read from `actions`, never the phase: a disconnect mid-Job drops the transport under a
  // moving blade, and those three verbs are the only ones a live Job ever offers.
  it("withholds it while a Job is in flight, whatever the phase says", () => {
    for (const live of [{ cancel: true }, { resume: true }, { confirm: true }]) {
      const status: CutStatus = { ...aStatus(), phase: "Idle", actions: { ...aStatus().actions, ...live } };
      expect(connectedControl(status, false)).toBeNull();
      expect(connectedControl(status, true)).toBeNull();
    }
  });
});
