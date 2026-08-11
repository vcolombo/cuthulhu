// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { groupDevices } from "./deviceList";
import type { DeviceInfo, PairedHostView } from "../ipc";

const aDevice = (): DeviceInfo => ({
  instance_id: "usb:mock",
  machine_id: "cameo5",
  transport: { Usb: { locator: "mock" } },
  candidate: false,
  host: null,
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
