// SPDX-License-Identifier: GPL-3.0-or-later
// @vitest-environment happy-dom
import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CutterStatusView, DeviceInfo, PairedHostView } from "../ipc";
import { IDLE_POLL_MS, LIVE_POLL_MS, useDevices } from "./useDevices";

const ipc = vi.hoisted(() => ({
  listDevices: vi.fn<() => Promise<DeviceInfo[]>>(),
  listHosts: vi.fn<() => Promise<PairedHostView[]>>(),
  cutterStatuses: vi.fn<() => Promise<CutterStatusView[]>>(),
}));

vi.mock("../ipc", () => ({
  listDevices: () => ipc.listDevices(),
  listHosts: () => ipc.listHosts(),
  cutterStatuses: () => ipc.cutterStatuses(),
  ipcErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

const HOST: PairedHostView = {
  id: "host-1", name: "Workshop Pi", address: "pi.local:7878", unreachable: null, protocol_side: null, last_seen: 1,
};
const DEVICE: DeviceInfo = {
  instance_id: "usb:pi:A", machine_id: "cameo5", transport: { Usb: { locator: "pi" } }, candidate: false, host: "host-1",
};

type Api = ReturnType<typeof useDevices>;

function Harness({ live, onPolled, api }: { live: boolean; onPolled?: (poll: { stale: boolean }) => void; api: { current: Api | null } }) {
  api.current = useDevices(live, onPolled);
  return createElement("div", null, api.current.devices.map((device) => device.instance_id).join(","), api.current.stale ? " stale" : "");
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
  });
}

describe("useDevices", () => {
  let root: Root;
  let host: HTMLDivElement;
  const api = { current: null as Api | null };

  beforeEach(() => {
    vi.useFakeTimers();
    ipc.listDevices.mockReset();
    ipc.listHosts.mockReset();
    ipc.cutterStatuses.mockReset();
    ipc.listDevices.mockResolvedValue([DEVICE]);
    ipc.listHosts.mockResolvedValue([HOST]);
    ipc.cutterStatuses.mockResolvedValue([]);
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  async function render(live: boolean, onPolled?: (poll: { stale: boolean }) => void) {
    await act(async () => {
      root.render(createElement(Harness, { live, onPolled, api }) as ReactNode);
    });
    await flush();
  }

  it("polls on one interval and skips a tick that is still in flight", async () => {
    await render(true);
    expect(ipc.listDevices).toHaveBeenCalledTimes(1);

    let release: (devices: DeviceInfo[]) => void = () => {};
    ipc.listDevices.mockImplementation(() => new Promise((resolve) => { release = resolve; }));
    await act(async () => { vi.advanceTimersByTime(LIVE_POLL_MS); });
    expect(ipc.listDevices).toHaveBeenCalledTimes(2);

    await act(async () => { vi.advanceTimersByTime(LIVE_POLL_MS); });
    expect(ipc.listDevices).toHaveBeenCalledTimes(2);

    await act(async () => { release([DEVICE]); });
    await flush();
  });

  it("keeps the previous rows when a read fails", async () => {
    const seen: boolean[] = [];
    await render(true, (poll) => seen.push(poll.stale));
    expect(api.current?.devices).toEqual([DEVICE]);
    expect(api.current?.stale).toBe(false);

    ipc.listDevices.mockRejectedValue(new Error("the device list could not be read"));
    await act(async () => { vi.advanceTimersByTime(LIVE_POLL_MS); });
    await flush();
    expect(api.current?.devices).toEqual([DEVICE]);
    expect(api.current?.stale).toBe(true);
    expect(api.current?.listError).toBe("the device list could not be read");
    expect(seen[seen.length - 1]).toBe(true);
  });

  it("does not start an interval when no host is paired", async () => {
    ipc.listHosts.mockResolvedValue([]);
    ipc.listDevices.mockResolvedValue([]);
    await render(true);
    expect(ipc.listDevices).toHaveBeenCalledTimes(1);
    await act(async () => { vi.advanceTimersByTime(IDLE_POLL_MS); });
    expect(ipc.listDevices).toHaveBeenCalledTimes(1);
  });

  it("waits out the idle interval until someone is looking", async () => {
    await render(false);
    expect(ipc.listDevices).toHaveBeenCalledTimes(1);
    await act(async () => { vi.advanceTimersByTime(LIVE_POLL_MS); });
    expect(ipc.listDevices).toHaveBeenCalledTimes(1);
    await act(async () => { vi.advanceTimersByTime(IDLE_POLL_MS - LIVE_POLL_MS); });
    expect(ipc.listDevices).toHaveBeenCalledTimes(2);
  });
});
