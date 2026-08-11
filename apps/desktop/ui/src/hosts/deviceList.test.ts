// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { deviceBadge, groupDevices } from "./deviceList";
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

  it("renders unknown status as attention, not gone and not ready", () => {
    const badge = deviceBadge(null);
    expect(badge.tone).toBe("attention");
  });

  it("flags AwaitingConfirmation-style states even though their phase differs from Idle", () => {
    const status: CutStatus = { ...aStatus(), phase: "AwaitingColorSwap", actions: { ...aStatus().actions, confirm: true } };
    expect(deviceBadge(status).tone).toBe("attention");
  });

  it("shows a running job as busy when only cancel is legal", () => {
    const status: CutStatus = { ...aStatus(), phase: "Sending", actions: { ...aStatus().actions, cancel: true } };
    expect(deviceBadge(status).tone).toBe("busy");
  });
});
