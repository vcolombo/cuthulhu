// SPDX-License-Identifier: GPL-3.0-or-later
// @vitest-environment happy-dom
import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DeviceInfo } from "../../ipc";
import type { InstalledPlan } from "./useCutPlan";
import type { Preset, PresetLookup } from "../viewmodel";
import { useCutDispatch } from "./useCutDispatch";

const ipc = vi.hoisted(() => ({
  cut: vi.fn<() => Promise<{ job_id: number; duplicate: boolean }>>(),
}));

vi.mock("../../ipc", () => ({
  cut: () => ipc.cut(),
  ipcErrorMessage: (e: unknown) =>
    e !== null && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e),
  ipcErrorCode: (e: unknown) =>
    e !== null && typeof e === "object" && "code" in e ? String((e as { code: unknown }).code) : null,
}));

type DispatchApi = ReturnType<typeof useCutDispatch>;

const plan: InstalledPlan = {
  grouping: "Color",
  weed: null,
  revision: "1",
  rows: [
    { key: "color:ff0000ff", shapeCount: 2, enabled: true, presetId: null, speed: null, force: null, repeatCount: null, nodeIds: [], starts: [], weed: [] },
    { key: "color:00ff00ff", shapeCount: 1, enabled: false, presetId: null, speed: null, force: null, repeatCount: null, nodeIds: [], starts: [], weed: [] },
  ],
  skippedNotCut: 0,
  travel: [],
};

const lookup: PresetLookup = { presets: [], loaded: true };

const htv: Preset = {
  id: "cameo5-htv",
  name: "Heat transfer",
  machine_id: "cameo5",
  settings: { speed: null, force: null, repeat_count: 1 },
  builtin: true,
};

function Host({
  api,
  onStalePlan,
  onError,
  onAccepted,
  installed = plan,
  presets = lookup,
}: {
  api: { current: DispatchApi | null };
  onStalePlan: () => void;
  onError: (msg: string) => void;
  onAccepted: (snapshot: { passes: { key: string }[] }) => void;
  installed?: InstalledPlan;
  presets?: PresetLookup;
}) {
  api.current = useCutDispatch({
    connected: { instance_id: "usb:mock" } as DeviceInfo,
    plan: installed,
    replanning: false,
    weedSettled: true,
    canSend: () => true,
    onStalePlan,
    onError,
    onAccepted,
    presetLookup: presets,
  });
  return null;
}

describe("useCutDispatch", () => {
  let root: Root;
  let el: HTMLDivElement;
  const api = { current: null as DispatchApi | null };
  const errors: string[] = [];
  const accepted: { passes: { key: string }[] }[] = [];
  let stale = 0;

  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  function render(): ReactNode {
    return createElement(Host, {
      api,
      onStalePlan: () => { stale += 1; },
      onError: (msg) => errors.push(msg),
      onAccepted: (snapshot) => accepted.push(snapshot),
    });
  }

  beforeEach(() => {
    ipc.cut.mockReset();
    api.current = null;
    errors.length = 0;
    accepted.length = 0;
    stale = 0;
    el = document.createElement("div");
    document.body.appendChild(el);
    root = createRoot(el);
  });

  afterEach(() => {
    act(() => root.unmount());
    el.remove();
  });

  it("shows the stale banner and does not toast the same refusal", async () => {
    ipc.cut.mockRejectedValue({ code: "stale_plan", message: "document changed since the cut was planned; replan" });
    await act(async () => { root.render(render()); });
    await act(async () => { api.current?.startCut(); });
    expect(stale).toBe(1);
    expect(errors).toEqual([]);
  });

  it("reports a cut refusal that is not a stale plan", async () => {
    ipc.cut.mockRejectedValue({ code: "plan_error", message: "no fonts are installed" });
    await act(async () => { root.render(render()); });
    await act(async () => { api.current?.startCut(); });
    expect(stale).toBe(0);
    expect(errors).toEqual(["no fonts are installed"]);
    expect(accepted).toEqual([]);
  });

  it("remembers every planned pass, including one that is switched off", async () => {
    ipc.cut.mockResolvedValue({ job_id: 4, duplicate: false });
    await act(async () => { root.render(render()); });
    await act(async () => { api.current?.startCut(); });
    expect(accepted).toEqual([{
      jobId: 4,
      grouping: "Color",
      passes: [
        { key: "color:ff0000ff", shapeCount: 2, enabled: true, name: "Red vinyl", swatch: "#ff0000" },
        { key: "color:00ff00ff", shapeCount: 1, enabled: false, name: "#00ff00", swatch: "#00ff00" },
      ],
    }]);
  });

  it("stores the preset name from the list held when the cut was accepted", async () => {
    ipc.cut.mockResolvedValue({ job_id: 4, duplicate: false });
    const presetPlan: InstalledPlan = {
      ...plan,
      grouping: "Preset",
      rows: [{ ...plan.rows[0], key: "preset:cameo5-htv" }],
    };
    await act(async () => {
      root.render(createElement(Host, {
        api,
        onStalePlan: () => { stale += 1; },
        onError: (msg) => errors.push(msg),
        onAccepted: (snapshot) => accepted.push(snapshot),
        installed: presetPlan,
        presets: { presets: [htv], loaded: true },
      }));
    });
    await act(async () => { api.current?.startCut(); });
    expect(accepted[0]?.passes[0]).toMatchObject({ name: "Heat transfer", swatch: null });
  });

  it("keeps the sheet's passes when the host had already accepted the cut", async () => {
    ipc.cut.mockResolvedValue({ job_id: 4, duplicate: true });
    await act(async () => { root.render(render()); });
    await act(async () => { api.current?.startCut(); });
    expect(accepted).toEqual([]);
  });
});
