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
  cancelCut: vi.fn<() => Promise<void>>(),
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
  },
  getDeviceState: () => ipc.getDeviceState(),
  listHosts: () => ipc.listHosts(),
  cancelCut: () => ipc.cancelCut(),
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

const HOST: PairedHostView = { id: "host-3", name: "Bench Pi", address: "bench.local:7878", unreachable: null };
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
      { key: "color:ff0000ff", shapeCount: 1, enabled: true },
      { key: "color:d39900ff", shapeCount: 1, enabled: true },
    ],
  };
}

function emit(jobId: number, next: CutStatus) {
  for (const listener of [...listeners]) listener({ payload: { job_id: jobId, kind: "StateChanged", status: next } });
}

describe("useJob", () => {
  let root: Root;
  let el: HTMLDivElement;
  const api = { current: null as JobApi | null };

  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  function render(): ReactNode {
    return createElement(() => {
      api.current = useJob(() => {});
      return null;
    });
  }

  beforeEach(() => {
    listeners.length = 0;
    ipc.getDeviceState.mockReset();
    ipc.listHosts.mockReset();
    ipc.cancelCut.mockReset();
    ipc.getDeviceState.mockResolvedValue(DISCONNECTED_STATUS);
    ipc.listHosts.mockResolvedValue([HOST]);
    ipc.cancelCut.mockResolvedValue(undefined);
    api.current = null;
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
    ipc.getDeviceState.mockResolvedValue(COMPLETED);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { emit(1, COMPLETED); });
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
      actions: { cut: false, cancel: true, resume: false, confirm: false },
      pass: { index: 0, total: 2 },
      sent: { sent: 0, total: 100 },
    });
    await act(async () => { emit(2, sending); });
    expect(api.current?.status).toEqual(sending);

    await act(async () => { emit(2, COMPLETED); });
    expect(api.current?.status).toEqual(COMPLETED);
  });

  it("accepts a remote cut's identical ending once the host has shown it active", async () => {
    ipc.getDeviceState.mockResolvedValue(COMPLETED);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { emit(1, COMPLETED); });
    await act(async () => { api.current?.remember(snapshot(0)); });
    expect(api.current?.status.phase).toBe("Sending");

    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.ended).toBeNull();

    const sending = status({
      phase: "Sending",
      actions: { cut: false, cancel: true, resume: false, confirm: false },
      pass: { index: 0, total: 1 },
      sent: { sent: 10, total: 100 },
    });
    ipc.getDeviceState.mockResolvedValue(sending);
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status).toEqual(sending);

    ipc.getDeviceState.mockResolvedValue(COMPLETED);
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status).toEqual(COMPLETED);
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
    await act(async () => { emit(1, COMPLETED); });

    let pending: Promise<void> | undefined;
    await act(async () => { pending = api.current?.refreshDeviceState(); });
    await act(async () => { api.current?.remember(snapshot(0)); });
    expect(api.current?.status.pass).toBeNull();

    const staleSending = status({
      phase: "Sending",
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

    ipc.getDeviceState.mockResolvedValue(COMPLETED);
    await act(async () => { await api.current?.refreshDeviceState(); });
    expect(api.current?.status.phase).toBe("Sending");
    expect(api.current?.status.ended).toBeNull();
  });

  it("keeps a failure that arrived before the cut was accepted", async () => {
    const failed = status({ phase: "Failed", error: "Timeout" });
    ipc.getDeviceState.mockResolvedValue(SWAP);
    await mount();
    await act(async () => { api.current?.attach(REMOTE); });
    await act(async () => { api.current?.remember(snapshot(1)); });
    await act(async () => { emit(1, COMPLETED); });
    await act(async () => {
      emit(2, status({ phase: "Sending", actions: { cut: false, cancel: true, resume: false, confirm: false } }));
      emit(2, failed);
    });
    await act(async () => { api.current?.remember(snapshot(2)); });
    expect(api.current?.status).toEqual(failed);

    ipc.getDeviceState.mockResolvedValue(COMPLETED);
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

  it("does not apply a detach refetch once the aim has moved again", async () => {
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
    await act(async () => {
      stateReads[2](SWAP);
      await pending;
    });
    expect(api.current?.status.phase).toBe("Disconnected");
    expect(api.current?.status.actions.resume).toBe(false);
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
    expect(api.current?.status).toEqual(SWAP);
  });
});
