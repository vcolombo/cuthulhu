// SPDX-License-Identifier: GPL-3.0-or-later
// @vitest-environment happy-dom
import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PlanCutResponse } from "../../ipc";
import type { WeedDraft } from "../viewmodel";
import { useCutPlan } from "./useCutPlan";

const ipc = vi.hoisted(() => ({
  planCut: vi.fn<(grouping: string, weed: unknown) => Promise<PlanCutResponse>>(),
  travelForOrder: vi.fn<() => Promise<[number, number, number, number][]>>(),
}));

vi.mock("../../ipc", () => ({
  planCut: (grouping: string, weed: unknown) => ipc.planCut(grouping, weed),
  travelForOrder: () => ipc.travelForOrder(),
  ipcErrorMessage: (e: unknown) =>
    e instanceof Error ? e.message : String((e as { message?: unknown } | null)?.message ?? e),
  ipcErrorCode: (e: unknown) =>
    e !== null && typeof e === "object" && "code" in e ? String((e as { code: unknown }).code) : null,
}));

type PlanApi = ReturnType<typeof useCutPlan>;

function Host({
  api,
  weed,
  onWeedChange,
  onError,
}: {
  api: { current: PlanApi | null };
  weed: WeedDraft | null;
  onWeedChange: (draft: WeedDraft) => void;
  onError: (msg: string) => void;
}) {
  api.current = useCutPlan({ weed, onWeedChange, ranges: null, onError });
  return null;
}

function planResponse(revision: string, key: string): PlanCutResponse {
  return {
    doc_revision: revision,
    skipped_not_cut: 0,
    travel: [],
    passes: [{ key, shape_count: 1, node_ids: [1], starts: [[0, 0]], weed: [] }],
  };
}

describe("useCutPlan", () => {
  let root: Root;
  let el: HTMLDivElement;
  const api = { current: null as PlanApi | null };
  const errors: string[] = [];
  const weedChanges: WeedDraft[] = [];

  // happy-dom does not set the flag React's act() looks for, so every update warns.
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  function render(): ReactNode {
    return createElement(Host, {
      api,
      weed: null,
      onWeedChange: (draft) => weedChanges.push(draft),
      onError: (msg) => errors.push(msg),
    });
  }

  beforeEach(() => {
    ipc.planCut.mockReset();
    ipc.travelForOrder.mockReset();
    ipc.travelForOrder.mockResolvedValue([]);
    api.current = null;
    errors.length = 0;
    weedChanges.length = 0;
    el = document.createElement("div");
    document.body.appendChild(el);
    root = createRoot(el);
  });

  afterEach(() => {
    act(() => root.unmount());
    el.remove();
  });

  it("installs the opening plan_cut and ignores one a newer replan has superseded", async () => {
    const pending: ((value: PlanCutResponse) => void)[] = [];
    ipc.planCut.mockImplementation(
      () => new Promise((resolve) => pending.push(resolve)),
    );

    await act(async () => {
      root.render(render());
    });
    expect(ipc.planCut).toHaveBeenCalledTimes(1);
    expect(ipc.planCut).toHaveBeenCalledWith("Color", null);
    expect(pending).toHaveLength(1);

    await act(async () => {
      api.current?.replan("Single");
    });
    expect(pending).toHaveLength(2);

    await act(async () => {
      pending[0](planResponse("old", "color:ff0000ff"));
    });
    expect(api.current?.plan).toBeNull();

    await act(async () => {
      pending[1](planResponse("new", "all"));
    });
    expect(api.current?.plan?.revision).toBe("new");
    expect(api.current?.plan?.grouping).toBe("Single");
    expect(api.current?.plan?.rows.map((row) => row.key)).toEqual(["all"]);
    expect(api.current?.replanning).toBe(false);
    expect(api.current?.stalePlan).toBe(false);
    expect(api.current?.weedSettled).toBe(true);
  });

  it("keeps the installed plan and restores its grouping when a replan fails", async () => {
    let resolveFirst: (value: PlanCutResponse) => void = () => {};
    ipc.planCut.mockImplementationOnce(
      () => new Promise((resolve) => {
        resolveFirst = resolve;
      }),
    );
    await act(async () => {
      root.render(render());
    });
    await act(async () => {
      resolveFirst(planResponse("kept", "color:ff0000ff"));
    });
    expect(api.current?.plan?.revision).toBe("kept");

    let rejectNext: (reason: unknown) => void = () => {};
    ipc.planCut.mockImplementationOnce(
      () => new Promise((_resolve, reject) => {
        rejectNext = reject;
      }),
    );
    await act(async () => {
      api.current?.setGrouping("Single");
      api.current?.replan("Single");
    });
    expect(api.current?.grouping).toBe("Single");
    expect(api.current?.replanning).toBe(true);

    await act(async () => {
      rejectNext({ code: "plan_error", message: "shape #2: no fonts are installed on this system" });
    });
    expect(api.current?.plan?.revision).toBe("kept");
    expect(api.current?.grouping).toBe("Color");
    expect(api.current?.replanning).toBe(false);
    expect(errors).toEqual(["shape #2: no fonts are installed on this system"]);
    // The installed plan had no weed, so the controls go back to an unticked border. The
    // spread is that restore: a failure must not leave the picker on a mode the plan is not.
    expect(weedChanges).toEqual([{ border: false, margin: "", lines: "None", spacing: "", clearance: "" }]);
  });

  it("plans the grouping now selected when a replan captured earlier runs late", async () => {
    let resolveFirst: (value: PlanCutResponse) => void = () => {};
    ipc.planCut.mockImplementationOnce(
      () => new Promise((resolve) => {
        resolveFirst = resolve;
      }),
    );
    await act(async () => {
      root.render(render());
    });
    await act(async () => {
      resolveFirst(planResponse("color", "color:ff0000ff"));
    });
    const held = api.current?.replan;
    expect(held).toBeTypeOf("function");

    let resolveSingle: (value: PlanCutResponse) => void = () => {};
    ipc.planCut.mockImplementationOnce(
      () => new Promise((resolve) => {
        resolveSingle = resolve;
      }),
    );
    await act(async () => {
      api.current?.setGrouping("Single");
      api.current?.replan("Single");
    });
    await act(async () => {
      resolveSingle(planResponse("single", "all"));
    });
    expect(api.current?.plan?.grouping).toBe("Single");

    let rejectHeld: (reason: unknown) => void = () => {};
    ipc.planCut.mockImplementationOnce(
      () => new Promise((_resolve, reject) => {
        rejectHeld = reject;
      }),
    );
    await act(async () => {
      held?.();
    });
    expect(ipc.planCut).toHaveBeenLastCalledWith("Single", null);

    await act(async () => {
      rejectHeld({ code: "plan_error", message: "late" });
    });
    expect(api.current?.plan?.revision).toBe("single");
    expect(api.current?.grouping).toBe("Single");
    expect(errors[errors.length - 1]).toBe("late");
  });
});
