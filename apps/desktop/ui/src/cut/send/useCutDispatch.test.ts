// SPDX-License-Identifier: GPL-3.0-or-later
// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DeviceInfo } from "../../ipc";
import type { InstalledPlan } from "./useCutPlan";
import { useCutDispatch } from "./useCutDispatch";

const ipc = vi.hoisted(() => ({
  cut: vi.fn(),
  resumeCut: vi.fn(),
  cancelCut: vi.fn(),
  confirmPassDone: vi.fn(),
}));

vi.mock("../../ipc", () => ({
  cut: (request: unknown) => ipc.cut(request),
  resumeCut: () => ipc.resumeCut(),
  cancelCut: () => ipc.cancelCut(),
  confirmPassDone: () => ipc.confirmPassDone(),
  ipcErrorMessage: (e: unknown) =>
    e !== null && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e),
  ipcErrorCode: (e: unknown) =>
    e !== null && typeof e === "object" && "code" in e ? String((e as { code: unknown }).code) : null,
}));

type DispatchApi = ReturnType<typeof useCutDispatch>;

const connected: DeviceInfo = {
  instance_id: "usb-1",
  machine_id: "cameo5",
  transport: { Usb: { locator: "1-2" } },
  candidate: false,
  host: null,
};

const plan: InstalledPlan = {
  grouping: "Color",
  weed: null,
  revision: "rev-1",
  rows: [{
    key: "color:ff0000ff",
    shapeCount: 1,
    nodeIds: [1],
    starts: [[0, 0]],
    weed: [],
    enabled: true,
    presetId: "vinyl",
    speed: 4,
    force: 8,
    repeatCount: 2,
  }],
  skippedNotCut: 0,
  travel: [],
};

type DispatchProps = {
  connected: DeviceInfo | null;
  plan: InstalledPlan | null;
  replanning: boolean;
  weedSettled: boolean;
  canSend: () => boolean;
  onStalePlan: () => void;
  onError: (msg: string) => void;
  refreshDeviceState: () => Promise<void>;
};

function Host({ api, props }: { api: { current: DispatchApi | null }; props: DispatchProps }) {
  api.current = useCutDispatch(props);
  return null;
}

describe("useCutDispatch", () => {
  let root: Root;
  let el: HTMLDivElement;
  const api = { current: null as DispatchApi | null };
  const errors: string[] = [];
  let stale = 0;
  let refreshes = 0;
  let props: DispatchProps;

  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  function render() {
    return createElement(Host, { api, props });
  }

  beforeEach(() => {
    ipc.cut.mockReset();
    ipc.resumeCut.mockReset();
    ipc.cancelCut.mockReset();
    ipc.confirmPassDone.mockReset();
    api.current = null;
    errors.length = 0;
    stale = 0;
    refreshes = 0;
    props = {
      connected,
      plan,
      replanning: false,
      weedSettled: true,
      canSend: () => true,
      onStalePlan: () => {
        stale += 1;
      },
      onError: (msg) => errors.push(msg),
      refreshDeviceState: async () => {
        refreshes += 1;
      },
    };
    el = document.createElement("div");
    document.body.appendChild(el);
    root = createRoot(el);
  });

  afterEach(() => {
    act(() => root.unmount());
    el.remove();
  });

  async function mount() {
    await act(async () => {
      root.render(render());
    });
  }

  it("does not send a cut while the rows on screen are not the plan that would be cut", async () => {
    const withheld: Partial<DispatchProps>[] = [
      { connected: null },
      { plan: null },
      { replanning: true },
      { weedSettled: false },
      { canSend: () => false },
    ];
    for (const patch of withheld) {
      props = { ...props, connected, plan, replanning: false, weedSettled: true, canSend: () => true, ...patch };
      await mount();
      await act(async () => {
        api.current?.startCut();
      });
    }
    expect(ipc.cut).not.toHaveBeenCalled();
  });

  it("sends the installed grouping, weed and rows, and tells a duplicate apart from a cut that started", async () => {
    await mount();
    let resolveCut: (value: { job_id: number; duplicate: boolean }) => void = () => {};
    ipc.cut.mockImplementationOnce(
      () => new Promise((resolve) => {
        resolveCut = resolve;
      }),
    );
    await act(async () => {
      api.current?.startCut();
    });
    expect(api.current?.cutInFlight).toBe(true);
    expect(api.current?.alreadyAccepted).toBe(false);
    expect(ipc.cut).toHaveBeenCalledWith({
      device_instance_id: "usb-1",
      doc_revision: "rev-1",
      grouping: "Color",
      weed: null,
      passes: [{
        key: "color:ff0000ff",
        enabled: true,
        preset_id: "vinyl",
        speed: 4,
        force: 8,
        repeat_count: 2,
      }],
    });

    await act(async () => {
      resolveCut({ job_id: 7, duplicate: true });
    });
    expect(api.current?.cutInFlight).toBe(false);
    // The host started nothing. A cutter that is not moving looks the same as one that is about
    // to, and only this flag says which.
    expect(api.current?.alreadyAccepted).toBe(true);

    ipc.cut.mockResolvedValueOnce({ job_id: 8, duplicate: false });
    await act(async () => {
      api.current?.startCut();
    });
    expect(api.current?.alreadyAccepted).toBe(false);
    expect(api.current?.cutInFlight).toBe(false);
  });

  it("shows the stale banner and does not toast the same refusal", async () => {
    await mount();
    ipc.cut.mockRejectedValueOnce({
      code: "stale_plan",
      message: "document changed since the cut was planned; replan",
    });
    await act(async () => {
      api.current?.startCut();
    });
    expect(stale).toBe(1);
    expect(errors).toEqual([]);
    expect(api.current?.cutInFlight).toBe(false);
    expect(api.current?.alreadyAccepted).toBe(false);
  });

  it("reports a cut refusal that is not a stale plan", async () => {
    await mount();
    ipc.cut.mockRejectedValueOnce({ code: "nothing_to_cut", message: "no enabled passes" });
    await act(async () => {
      api.current?.startCut();
    });
    expect(stale).toBe(0);
    expect(errors).toEqual(["no enabled passes"]);
    expect(api.current?.cutInFlight).toBe(false);
  });

  it("refreshes after resume, cancel and confirm land, and reports a refusal without refreshing", async () => {
    await mount();
    const commands: [keyof typeof ipc, () => void][] = [
      ["resumeCut", () => api.current?.resume()],
      ["cancelCut", () => api.current?.cancel()],
      ["confirmPassDone", () => api.current?.confirmPassDone()],
    ];
    for (const [method, run] of commands) {
      ipc[method].mockResolvedValueOnce(undefined);
      await act(async () => {
        run();
      });
    }
    expect(refreshes).toBe(3);
    expect(errors).toEqual([]);

    for (const [method, run] of commands) {
      ipc[method].mockRejectedValueOnce({ message: "the cutter is busy" });
      await act(async () => {
        run();
      });
    }
    expect(refreshes).toBe(3);
    expect(errors).toEqual(["the cutter is busy", "the cutter is busy", "the cutter is busy"]);
  });
});
