// SPDX-License-Identifier: GPL-3.0-or-later
// @vitest-environment happy-dom
import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CutStatus, DeviceEvent, DeviceInfo, PairedHostView } from "../ipc";
import { DISCONNECTED_STATUS } from "../ipc";
import type { JobSnapshot } from "../cut/job/model";
import { useJob } from "./useJob";

type Listener = (event: { payload: DeviceEvent }) => void;

const listeners: Listener[] = [];

const ipc = vi.hoisted(() => ({
  getDeviceState: vi.fn<() => Promise<CutStatus>>(),
  listHosts: vi.fn<() => Promise<PairedHostView[]>>(),
  cancelCut: vi.fn<(target?: { host: string | null; instanceId: string }) => Promise<void>>(),
  resumeCut: vi.fn<() => Promise<void>>(),
  confirmPassDone: vi.fn<() => Promise<void>>(),
}));

vi.mock("../ipc", () => ({
  DISCONNECTED_STATUS: {
    phase: "Disconnected",
    ended: null,
    actions: { cut: false, cancel: false, resume: false, confirm: false },
    pass: null,
    sent: null,
    error: null,
    jobId: 0,
  },
  getDeviceState: () => ipc.getDeviceState(),
  listHosts: () => ipc.listHosts(),
  cancelCut: (target?: { host: string | null; instanceId: string }) => ipc.cancelCut(target),
  resumeCut: () => ipc.resumeCut(),
  confirmPassDone: () => ipc.confirmPassDone(),
  ipcErrorMessage: (e: unknown) => String(e),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, cb: Listener) => {
    listeners.push(cb);
    return Promise.resolve(() => {
      const index = listeners.indexOf(cb);
      if (index >= 0) listeners.splice(index, 1);
    });
  },
}));

type JobApi = ReturnType<typeof useJob>;

const HOST: PairedHostView = { id: "host-3", name: "Bench Pi", address: "bench.local:7878", unreachable: null, protocol_side: null };
const REMOTE = { instance_id: "usb:pi:C", machine_id: "cameo5", host: "host-3" } as DeviceInfo;

function status(patch: Partial<CutStatus> & Pick<CutStatus, "phase">): CutStatus {
  return {
    ...DISCONNECTED_STATUS,
    ended: null,
    actions: { cut: false, cancel: false, resume: false, confirm: false },
    pass: null,
    sent: null,
    error: null,
    ...patch,
  };
}

const COMPLETED = status({ phase: "Idle", ended: "Completed", actions: { cut: true, cancel: false, resume: false, confirm: false } });
const SWAP = status({
  phase: "AwaitingColorSwap",
  actions: { cut: false, cancel: true, resume: true, confirm: false },
  pass: { index: 1, total: 2 },
});

function snapshot(jobId: number): JobSnapshot {
  return {
    jobId,
    grouping: "Color",
    passes: [
      { key: "color:ff0000ff", shapeCount: 1, enabled: true, name: "Red vinyl", swatch: "#ff0000" },
      { key: "color:d39900ff", shapeCount: 1, enabled: true, name: "Gold vinyl", swatch: "#d39900" },
    ],
  };
}

function emit(jobId: number, next: CutStatus) {
  const status = { ...next, jobId };
  for (const listener of [...listeners]) listener({ payload: { job_id: jobId, kind: "StateChanged", status } });
}

function done(jobId: number): CutStatus {
  return status({
    phase: "Idle",
    ended: "Completed",
    jobId,
    actions: { cut: true, cancel: false, resume: false, confirm: false },
  });
}

describe("useJob", () => {
  let root: Root;
  let el: HTMLDivElement;
  const api = { current: null as JobApi | null };
  const errors: string[] = [];

  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  function render(): ReactNode {
    return createElement(() => {
      api.current = useJob((message) => errors.push(message));
      return null;
    });
  }

  beforeEach(() => {
    listeners.length = 0;
    ipc.getDeviceState.mockReset();
    ipc.listHosts.mockReset();
    ipc.cancelCut.mockReset();
    ipc.resumeCut.mockReset();
    ipc.confirmPassDone.mockReset();
    ipc.getDeviceState.mockResolvedValue(DISCONNECTED_STATUS);
    ipc.listHosts.mockResolvedValue([HOST]);
    ipc.cancelCut.mockResolvedValue(undefined);
    ipc.resumeCut.mockResolvedValue(undefined);
    ipc.confirmPassDone.mockResolvedValue(undefined);
    api.current = null;
    errors.length = 0;
    el = document.createElement("div");
    document.body.appendChild(el);
    root = createRoot(el);
  });

  afterEach(() => {
    act(() => root.unmount());
    el.remove();
  });

  async function mount() {
    await act(async () => { root.render(render()); });
  }

  it("replaces a finished job when the next Sending event has not arrived", async () => {
    ipc.getDeviceState.mockResolvedValue(done(1));
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, done(1)); });
    await act(async () => { api.current?.remember(snapshot(2)); });

    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.ended).toBeNull();
    expect(api.current?.status.actions.cancel).toBe(true);

    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.ended).toBeNull();

    await act(async () => { emit(1, status({ phase: "Failed", error: "Timeout" })); });
    expect(api.current?.status.phase).toBe("Sending");

    const sending = status({
      phase: "Sending",
      jobId: 2,
      actions: { cut: false, cancel: true, resume: false, confirm: false },
      pass: { index: 0, total: 2 },
      sent: { sent: 0, total: 100 },
    });
    await act(async () => { emit(2, sending); });
    expect(api.current?.status).toEqual(sending);

    await act(async () => { emit(2, done(2)); });
    expect(api.current?.status).toEqual(done(2));
  });

  it("shows a remote cut that ends the same way as the one before it", async () => {
    ipc.getDeviceState.mockResolvedValue(done(1));
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, done(1)); });
    await act(async () => { api.current?.remember(snapshot(0)); });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.jobId).toBe(0);

    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.ended).toBeNull();

    const sending = status({
      phase: "Sending",
      jobId: 2,
      actions: { cut: false, cancel: true, resume: false, confirm: false },
      pass: { index: 0, total: 1 },
      sent: { sent: 10, total: 100 },
    });
    ipc.getDeviceState.mockResolvedValue(sending);
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status).toEqual(sending);

    ipc.getDeviceState.mockResolvedValue(done(2));
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status).toEqual(done(2));
  });

  it("keeps two jobs that finish the same way", async () => {
    ipc.getDeviceState.mockResolvedValue(done(4));
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(4)); });
    await act(async () => { emit(4, done(4)); });
    expect(api.current?.status).toEqual(done(4));

    await act(async () => { api.current?.remember(snapshot(5)); });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.jobId).toBe(5);
    await act(async () => { emit(5, done(5)); });
    expect(api.current?.status).toEqual(done(5));
    expect(api.current?.offlineHost).toBeNull();
  });

  it("drops a late status from the previous job", async () => {
    ipc.getDeviceState.mockResolvedValue(done(4));
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(4)); });
    await act(async () => { emit(4, done(4)); });
    await act(async () => { api.current?.remember(snapshot(5)); });
    const sending = status({
      phase: "Sending",
      jobId: 5,
      actions: { cut: false, cancel: true, resume: false, confirm: false },
    });
    await act(async () => { emit(5, sending); });
    await act(async () => { emit(4, done(4)); });
    expect(api.current?.status).toEqual(sending);

    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status).toEqual(sending);
    expect(api.current?.offlineHost).toBeNull();
  });

  it("does not let a poll that started before accept clear the previous ending", async () => {
    let resolveRead: (next: CutStatus) => void = () => {};
    let reads = 0;
    ipc.getDeviceState.mockImplementation(() => {
      reads += 1;
      if (reads === 1) return new Promise((resolve) => { resolveRead = resolve; });
      return new Promise(() => {});
    });
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { emit(1, done(1)); });

    let pending: Promise<void> | undefined;
    await act(async () => { pending = api.current?.refreshDeviceState(); });
    await act(async () => { api.current?.remember(snapshot(0)); });
    expect(api.current?.status.pass).toBeNull();
    expect(api.current?.status.ended).toBeNull();

    const staleSending = status({
      phase: "Sending",
      jobId: 1,
      actions: { cut: false, cancel: true, resume: false, confirm: false },
      pass: { index: 0, total: 1 },
      sent: { sent: 10, total: 100 },
    });
    await act(async () => {
      resolveRead(staleSending);
      await pending;
    });
    expect(api.current?.status.pass).toBeNull();
    expect(api.current?.status.ended).toBeNull();

    ipc.getDeviceState.mockResolvedValue(done(1));
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.ended).toBeNull();
  });

  it("keeps a failure that arrived before the cut was accepted", async () => {
    const failed = status({ phase: "Failed", error: "Timeout", jobId: 2 });
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, done(1)); });
    await act(async () => {
      emit(2, status({ phase: "Sending", actions: { cut: false, cancel: true, resume: false, confirm: false } }));
      emit(2, failed);
    });
    await act(async () => { api.current?.remember(snapshot(2)); });
    expect(api.current?.status).toEqual(failed);

    ipc.getDeviceState.mockResolvedValue(done(1));
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status).toEqual(failed);
  });

  it("drops a remote swap when the aim leaves, even if a poll still has it", async () => {
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, SWAP); });
    expect(api.current?.status.actions.resume).toBe(true);

    let releaseState: (next: CutStatus) => void = () => {};
    let releaseHosts: (hosts: PairedHostView[]) => void = () => {};
    ipc.getDeviceState.mockImplementation(() => new Promise((resolve) => { releaseState = resolve; }));
    ipc.listHosts.mockImplementation(() => new Promise((resolve) => { releaseHosts = resolve; }));
    const pending = api.current!.refreshDeviceState();
    ipc.getDeviceState.mockResolvedValue(DISCONNECTED_STATUS);
    ipc.listHosts.mockResolvedValue([HOST]);
    await act(async () => { api.current?.attach(null); });
    expect(api.current?.status.phase).toBe("Disconnected");
    expect(api.current?.status.actions.resume).toBe(false);

    await act(async () => {
      releaseHosts([HOST]);
      releaseState(SWAP);
      await pending;
    });
    expect(api.current?.status.phase).toBe("Disconnected");
    expect(api.current?.status.ended).toBeNull();
    expect(api.current?.offlineHost).toBeNull();
  });

  it("does not apply a detach refetch after the aim leaves and comes back to none", async () => {
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, SWAP); });

    const stateReads: Array<(next: CutStatus) => void> = [];
    ipc.getDeviceState.mockImplementation(() => new Promise((resolve) => { stateReads.push(resolve); }));
    let releaseHosts: (hosts: PairedHostView[]) => void = () => {};
    ipc.listHosts.mockImplementation(() => new Promise((resolve) => { releaseHosts = resolve; }));
    const pending = api.current!.refreshDeviceState();

    await act(async () => { api.current?.attach(null); });
    await act(async () => { stateReads[1](DISCONNECTED_STATUS); });
    expect(api.current?.status.phase).toBe("Disconnected");

    await act(async () => {
      releaseHosts([HOST]);
      stateReads[0](SWAP);
    });
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.attach(null); });
    await act(async () => {
      stateReads[2](SWAP);
      await pending;
    });
    expect(api.current?.status.phase).toBe("Disconnected");
    expect(api.current?.status.actions.resume).toBe(false);
  });

  it("shows a smaller job id after the aimed cutter changes", async () => {
    const other = { ...REMOTE, instance_id: "usb:pi:D" } as DeviceInfo;
    const first = done(5);
    const second = status({
      phase: "Sending",
      jobId: 2,
      actions: { cut: false, cancel: true, resume: false, confirm: false },
    });
    ipc.getDeviceState.mockResolvedValue(first);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(5)); });
    await act(async () => { emit(5, first); });
    expect(api.current?.status.jobId).toBe(5);

    ipc.getDeviceState.mockResolvedValue(second);
    await act(async () => { api.current?.attach(other); });
    await act(async () => { emit(2, second); });
    expect(api.current?.status).toEqual(second);
  });

  it("does not let the previous cutter's poll replace the one now aimed on the same host", async () => {
    const other = { ...REMOTE, instance_id: "usb:pi:D" } as DeviceInfo;
    const idle = status({ phase: "Idle", actions: { cut: true, cancel: false, resume: false, confirm: false } });
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, SWAP); });

    let releaseState: (next: CutStatus) => void = () => {};
    let releaseHosts: (hosts: PairedHostView[]) => void = () => {};
    ipc.getDeviceState.mockImplementation(() => new Promise((resolve) => { releaseState = resolve; }));
    ipc.listHosts.mockImplementation(() => new Promise((resolve) => { releaseHosts = resolve; }));
    const stale = api.current!.refreshDeviceState();

    ipc.getDeviceState.mockResolvedValue(idle);
    ipc.listHosts.mockResolvedValue([HOST]);
    await act(async () => { api.current?.attach(other); });
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status).toEqual(idle);

    await act(async () => {
      releaseHosts([HOST]);
      releaseState(SWAP);
      await stale;
    });
    expect(api.current?.status).toEqual(idle);
  });

  it("refetches when a same-host switch invalidates the poll already in flight", async () => {
    const other = { ...REMOTE, instance_id: "usb:pi:D" } as DeviceInfo;
    const idle = status({ phase: "Idle", actions: { cut: true, cancel: false, resume: false, confirm: false } });
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, SWAP); });

    let releaseState: (next: CutStatus) => void = () => {};
    let releaseHosts: (hosts: PairedHostView[]) => void = () => {};
    ipc.getDeviceState.mockImplementation(() => new Promise((resolve) => { releaseState = resolve; }));
    ipc.listHosts.mockImplementation(() => new Promise((resolve) => { releaseHosts = resolve; }));
    const pending = api.current!.refreshDeviceState();

    let releaseFresh: (next: CutStatus) => void = () => {};
    ipc.getDeviceState.mockImplementation(() => new Promise((resolve) => { releaseFresh = resolve; }));
    ipc.listHosts.mockResolvedValue([HOST]);
    await act(async () => { api.current?.attach(other); });
    await act(async () => {
      releaseHosts([HOST]);
      releaseState(SWAP);
    });
    expect(api.current?.status.phase).toBe("AwaitingColorSwap");

    await act(async () => {
      releaseFresh(idle);
      await pending;
    });
    expect(api.current?.status).toEqual(idle);
  });

  it("does not cancel a cut that finished while the host was unreachable", async () => {
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, SWAP); });
    ipc.listHosts.mockResolvedValue([{ ...HOST, unreachable: "the host could not be reached" }]);
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.offlineHost?.id).toBe("host-3");

    act(() => { api.current?.cancel(); });
    ipc.listHosts.mockResolvedValue([HOST]);
    ipc.getDeviceState.mockResolvedValue(COMPLETED);
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(ipc.cancelCut).not.toHaveBeenCalled();
    expect(api.current?.status).toEqual(COMPLETED);
    expect(api.current?.offlineHost).toBeNull();
  });

  it("shows a cut whose job id restarted after the host dropped", async () => {
    ipc.getDeviceState.mockResolvedValue(done(5));
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(5)); });
    await act(async () => { emit(5, done(5)); });

    // A finished job does not freeze, but the poll still records that the host dropped.
    act(() => {
      api.current?.applyPoll({
        hosts: [{ ...HOST, unreachable: "the host could not be reached" }],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: done(5) }],
        stale: false,
      });
    });
    expect(api.current?.status).toEqual(done(5));
    expect(api.current?.offlineHost).toBeNull();

    // The daemon process came back numbering from 1. The accept still reports no id.
    ipc.listHosts.mockResolvedValue([HOST]);
    await act(async () => { api.current?.remember(snapshot(0)); });
    const sending = status({
      phase: "Sending",
      jobId: 1,
      actions: { cut: false, cancel: true, resume: false, confirm: false },
      pass: { index: 0, total: 2 },
      sent: { sent: 10, total: 100 },
    });
    ipc.getDeviceState.mockResolvedValue(sending);
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status).toEqual(sending);

    const swap = { ...SWAP, jobId: 1 };
    act(() => {
      api.current?.applyPoll({
        hosts: [HOST],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: swap }],
        stale: false,
      });
    });
    expect(api.current?.status).toEqual(swap);
    expect(api.current?.status.actions.resume).toBe(true);
  });

  it("still drops a lower job id when the host never dropped", async () => {
    ipc.getDeviceState.mockResolvedValue(done(5));
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(5)); });
    await act(async () => { emit(5, done(5)); });
    await act(async () => { api.current?.remember(snapshot(0)); });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.jobId).toBe(0);

    const sending = status({
      phase: "Sending",
      jobId: 1,
      actions: { cut: false, cancel: true, resume: false, confirm: false },
    });
    ipc.getDeviceState.mockResolvedValue(sending);
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.jobId).toBe(0);

    await act(async () => { emit(1, sending); });
    expect(api.current?.status.jobId).toBe(0);
  });

  it("sends a queued cancel when the recovered job can still be cancelled", async () => {
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, SWAP); });
    ipc.listHosts.mockResolvedValue([{ ...HOST, unreachable: "the host could not be reached" }]);
    await act(async () => { await api.current?.refreshDeviceState(); });
    act(() => { api.current?.cancel(); });
    ipc.listHosts.mockResolvedValue([HOST]);
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(ipc.cancelCut).toHaveBeenCalledOnce();
    expect(ipc.cancelCut).toHaveBeenCalledWith({ host: "host-3", instanceId: "usb:pi:C" });
    expect(api.current?.status).toEqual({ ...SWAP, jobId: 1 });
  });

  it("names the same cutter when a poll delivers a cancel that was queued offline", async () => {
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, SWAP); });
    act(() => {
      api.current?.applyPoll({
        hosts: [{ ...HOST, unreachable: "the host could not be reached" }],
        statuses: [],
        stale: false,
      });
    });
    expect(api.current?.offlineHost?.id).toBe("host-3");
    act(() => { api.current?.cancel(); });
    act(() => {
      api.current?.applyPoll({
        hosts: [HOST],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: SWAP }],
        stale: false,
      });
    });
    expect(ipc.cancelCut).toHaveBeenCalledWith({ host: "host-3", instanceId: "usb:pi:C" });
  });

  it("shows a poll of the aimed cutter when no job is watched", async () => {
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    const ready = status({
      phase: "Idle",
      actions: { cut: true, cancel: false, resume: false, confirm: false },
      jobId: 0,
    });
    act(() => {
      api.current?.applyPoll({
        hosts: [HOST],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: ready }],
        stale: false,
      });
    });
    expect(api.current?.status).toEqual(ready);
    expect(api.current?.tracking).toBe(false);
    expect(api.current?.offlineHost).toBeNull();

    const busy = status({
      phase: "Sending",
      actions: { cut: false, cancel: true, resume: false, confirm: false },
      jobId: 4,
    });
    act(() => {
      api.current?.applyPoll({
        hosts: [HOST],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: busy }],
        stale: false,
      });
    });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.actions.cut).toBe(false);

    act(() => {
      api.current?.applyPoll({
        hosts: [HOST],
        statuses: [{ host: HOST.id, instance_id: "usb:pi:D", status: ready }],
        stale: false,
      });
    });
    expect(api.current?.status.phase).toBe("Sending");

    act(() => {
      api.current?.applyPoll({ hosts: [HOST], statuses: [], stale: false });
    });
    expect(api.current?.status.phase).toBe("Sending");

    act(() => {
      api.current?.applyPoll({
        hosts: [HOST],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: ready }],
        stale: true,
      });
    });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.offlineHost).toBeNull();

    act(() => {
      api.current?.applyPoll({
        hosts: [{ ...HOST, unreachable: "the host could not be reached" }],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: ready }],
        stale: false,
      });
    });
    expect(api.current?.offlineHost).toBeNull();
    expect(api.current?.status.phase).toBe("Sending");
  });

  it("leaves a finished job on screen when the next poll is a different reading", async () => {
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, done(1)); });
    expect(api.current?.status.ended).toBe("Completed");
    expect(api.current?.tracking).toBe(true);

    const plain = status({
      phase: "Idle",
      actions: { cut: true, cancel: false, resume: false, confirm: false },
      jobId: 0,
    });
    const someoneElse = status({
      phase: "Sending",
      actions: { cut: false, cancel: true, resume: false, confirm: false },
      jobId: 9,
    });
    act(() => {
      api.current?.applyPoll({
        hosts: [HOST],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: someoneElse }],
        stale: false,
      });
    });
    expect(api.current?.status.ended).toBe("Completed");
    expect(api.current?.status.jobId).toBe(1);

    act(() => { api.current?.dismiss(); });
    act(() => {
      api.current?.applyPoll({
        hosts: [HOST],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: plain }],
        stale: false,
      });
    });
    expect(api.current?.tracking).toBe(false);
    expect(api.current?.status).toEqual(plain);
  });

  it("does not let a poll in the same turn as remember cover the new job", async () => {
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, done(1)); });
    await act(async () => { api.current?.dismiss(); });

    const idle = status({
      phase: "Idle",
      actions: { cut: true, cancel: false, resume: false, confirm: false },
      jobId: 0,
    });
    await act(async () => {
      api.current?.remember(snapshot(2));
      api.current?.applyPoll({
        hosts: [HOST],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: idle }],
        stale: false,
      });
    });
    expect(api.current?.tracking).toBe(true);
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.jobId).toBe(2);
    expect(api.current?.offlineHost).toBeNull();

    await act(async () => {
      api.current?.applyPoll({
        hosts: [{ ...HOST, unreachable: "the host could not be reached" }],
        statuses: [{ host: HOST.id, instance_id: REMOTE.instance_id, status: idle }],
        stale: false,
      });
    });
    expect(api.current?.offlineHost?.id).toBe("host-3");
    expect(api.current?.status.phase).toBe("Sending");
  });

  it("does not freeze a job when the poll has no status for that cutter yet", async () => {
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, SWAP); });
    act(() => {
      api.current?.applyPoll({ hosts: [HOST], statuses: [], stale: false });
    });
    expect(api.current?.offlineHost).toBeNull();
    expect(api.current?.status.phase).toBe("AwaitingColorSwap");
  });

  it("refreshes after resume, cancel and confirm land, and reports a refusal without refreshing", async () => {
    await mount();
    ipc.getDeviceState.mockClear();
    const commands: [typeof ipc.resumeCut, () => void][] = [
      [ipc.resumeCut, () => api.current?.resume()],
      [ipc.cancelCut, () => api.current?.cancel()],
      [ipc.confirmPassDone, () => api.current?.confirmPassDone()],
    ];
    for (const [mock, run] of commands) {
      mock.mockResolvedValueOnce(undefined);
      await act(async () => {
        run();
      });
    }
    expect(ipc.getDeviceState).toHaveBeenCalledTimes(3);
    expect(errors).toEqual([]);

    ipc.getDeviceState.mockClear();
    for (const [mock, run] of commands) {
      mock.mockRejectedValueOnce("the cutter is busy");
      await act(async () => {
        run();
      });
    }
    expect(ipc.getDeviceState).not.toHaveBeenCalled();
    expect(errors).toEqual(["the cutter is busy", "the cutter is busy", "the cutter is busy"]);
  });
});
