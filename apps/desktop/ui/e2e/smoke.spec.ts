// SPDX-License-Identifier: GPL-3.0-or-later
import { test, expect } from "@playwright/test";

// Minimal in-memory fake Tauri backend. Runs inside the page (via addInitScript, so it
// can't close over anything outside itself) and mirrors the JSON shape produced by
// crates/document's Document::snapshot_json() — see App.tsx's DocSnapshot/buildScene,
// which is what actually parses this on the JS side.
function installMockTauri(opts?: { seedTwoColorRects?: boolean }) {
  type Style = { stroke: number | null; fill: number | null };
  type Node = { id: number; kind: unknown; transform: number[]; style: Style; children: number[] };
  type Doc = {
    nodes: Record<number, Node>;
    root: number;
    artboard: { x: number; y: number; w: number; h: number };
    machine: { id: string; name: string; width_mm: number; height_mm: number } | null;
  };

  const machines = [
    { id: "cameo5", name: "Silhouette Cameo 5 Alpha", width_mm: 330, height_mm: 3000 },
    { id: "puma", name: "GCC Puma IV", width_mm: 600, height_mm: 5000 },
  ];

  // Mirrors document::Style::default() — a freshly-added shape has an opaque black
  // stroke and is cuttable by default.
  const DEFAULT_STYLE: Style = { stroke: 0x000000ff, fill: null };

  let nextId = 1;
  const freshDoc = (): Doc => {
    const rootId = nextId++;
    return {
      nodes: { [rootId]: { id: rootId, kind: "Layer", transform: [1, 0, 0, 1, 0, 0], style: { stroke: null, fill: null }, children: [] } },
      root: rootId,
      artboard: { x: 0, y: 0, w: 330, h: 3000 },
      machine: null,
    };
  };
  let doc = freshDoc();
  let saved: Doc | null = null;

  // Seed two differently-stroked rects synchronously (bypassing invoke) so the doc is
  // already populated by the time App.tsx's mount effect calls snapshot() — avoids a
  // race between an async seed and React's first fetch.
  if (opts?.seedTwoColorRects) {
    const redId = nextId++;
    doc.nodes[redId] = {
      id: redId,
      kind: { Shape: { Rect: { x: 0, y: 0, w: 10, h: 10 } } },
      transform: [1, 0, 0, 1, 0, 0],
      style: { stroke: 0xff0000ff, fill: null },
      children: [],
    };
    const greenId = nextId++;
    doc.nodes[greenId] = {
      id: greenId,
      kind: { Shape: { Rect: { x: 20, y: 0, w: 10, h: 10 } } },
      transform: [1, 0, 0, 1, 0, 0],
      style: { stroke: 0x00ff00ff, fill: null },
      children: [],
    };
    doc.nodes[doc.root].children.push(redId, greenId);
  }

  // Bumped on every doc mutation; string-compared against CutRequest.doc_revision
  // to emulate the real stale_plan check (device.rs's prepare_cut).
  let revision = 0;

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    new_doc: () => {
      doc = freshDoc();
      revision++;
      return JSON.stringify(doc);
    },
    snapshot: () => JSON.stringify(doc),
    add_primitive: (a) => {
      const id = nextId++;
      const style = a.stroke !== undefined ? { stroke: a.stroke as number | null, fill: null } : DEFAULT_STYLE;
      doc.nodes[id] = { id, kind: { Shape: a.kind }, transform: [1, 0, 0, 1, 0, 0], style, children: [] };
      doc.nodes[a.parent as number].children.push(id);
      revision++;
      return {};
    },
    add_text: (a) => {
      const id = nextId++;
      doc.nodes[id] = { id, kind: { Shape: { Path: { d: "" } } }, transform: [1, 0, 0, 1, 0, 0], style: DEFAULT_STYLE, children: [] };
      doc.nodes[a.parent as number].children.push(id);
      revision++;
      return {};
    },
    commit_transform: (a) => {
      const m = a.m as number[];
      for (const id of a.ids as number[]) {
        const t = doc.nodes[id]?.transform;
        if (t) {
          t[4] += m[4];
          t[5] += m[5];
        }
      }
      return {};
    },
    delete: (a) => {
      for (const id of a.ids as number[]) {
        delete doc.nodes[id];
        for (const n of Object.values(doc.nodes)) n.children = n.children.filter((c) => c !== id);
      }
      revision++;
      return {};
    },
    reorder: () => {
      revision++;
      return {};
    },
    undo: () => null,
    redo: () => null,
    boolean_op: () => {
      revision++;
      return {};
    },
    import_svg: () => {
      revision++;
      return [{}, []];
    },
    save_project: () => {
      saved = JSON.parse(JSON.stringify(doc));
      return null;
    },
    load_project: () => {
      if (saved) doc = JSON.parse(JSON.stringify(saved));
      revision++;
      return JSON.stringify(doc);
    },
    set_machine: (a) => {
      const m = machines.find((p) => p.id === a.machineId);
      if (!m) throw new Error("unknown machine");
      doc.machine = m;
      doc.artboard = { x: 0, y: 0, w: m.width_mm, h: m.height_mm };
      revision++;
      return null;
    },
    list_machines: () => machines,
  };

  // --- device / cut / preset mock: mirrors apps/desktop/src/device.rs's validation
  // order and driver-core::manager's DeviceState/DeviceEvent shapes closely enough to
  // drive the cut dialog through a real state machine. ---

  type DeviceInfo = { instance_id: string; machine_id: string; transport: unknown; candidate: boolean };
  type DeviceState = unknown;
  type DeviceEvent = { job_id: number; kind: unknown };

  const devices: DeviceInfo[] = [
    { instance_id: "usb:mock", machine_id: "cameo5", transport: { Usb: { locator: "mock" } }, candidate: false },
    { instance_id: "serial:/dev/mock0", machine_id: "puma", transport: { Serial: { path: "/dev/mock0", baud: 9600 } }, candidate: true },
  ];

  let connected: DeviceInfo | null = null;
  let deviceState: DeviceState = "Disconnected";
  let nextJobId = 1;
  let jobId: number | null = null;
  let planPasses: { color: number | null; enabled: boolean }[] = [];

  function ipcError(code: string, message: string) {
    return { code, message };
  }

  function planFromDoc() {
    // Mirrors crates/cutplan/src/passes.rs's plan_passes: preorder walk, group Shape
    // leaf nodes by full stroke color (0-alpha counts as no stroke), first-seen order.
    const byColor = new Map<number, { color: number; node_ids: number[] }>();
    let skipped = 0;
    const walk = (id: number) => {
      const n = doc.nodes[id];
      if (!n) return;
      const isShape = typeof n.kind === "object" && n.kind !== null && "Shape" in (n.kind as object);
      if (isShape) {
        const stroke = n.style.stroke;
        if (stroke === null || stroke === undefined || (stroke & 0xff) === 0) {
          skipped++;
        } else {
          const existing = byColor.get(stroke);
          if (existing) existing.node_ids.push(id);
          else byColor.set(stroke, { color: stroke, node_ids: [id] });
        }
      }
      for (const c of n.children) walk(c);
    };
    walk(doc.root);
    const passes = [...byColor.values()].map((p) => ({ color: p.color, shape_count: p.node_ids.length, node_ids: p.node_ids }));
    return { passes, skipped_no_stroke: skipped, doc_revision: String(revision), travel: [] as [number, number, number, number][] };
  }

  // Mirrors @tauri-apps/api/event's listen()/transformCallback() plumbing: listen()
  // calls transformCallback(handler) to get a numeric id (stored in callbacksById),
  // then invoke("plugin:event|listen", {event, handler: id}) associates that id with
  // an event name (eventNameToIds). Emitting calls the stored callback directly, like
  // the real event bridge thread does via window["_" + id](payload).
  const callbacksById = new Map<number, (e: unknown) => void>();
  const eventNameToIds = new Map<string, number[]>();
  let nextCallbackId = 1;

  function emit(kind: unknown) {
    const ev: DeviceEvent = { job_id: jobId ?? 0, kind };
    for (const id of eventNameToIds.get("device-event") ?? []) {
      callbacksById.get(id)?.({ event: "device-event", id, payload: ev });
    }
  }

  // Drives the scripted pass sequence for one pass, then either pauses at
  // WaitingForColorSwap (more enabled passes remain) or completes the job — matching
  // execute_cut's documented behavior of blocking until the next pause point.
  function runPass(passIndex: number, enabledIndices: number[]) {
    const total = 100;
    deviceState = { Transmitting: { job_id: jobId, pass_index: passIndex, submitted_bytes: 0, total_bytes: total } };
    emit({ StateChanged: deviceState });
    deviceState = { Transmitting: { job_id: jobId, pass_index: passIndex, submitted_bytes: total, total_bytes: total } };
    emit({ Progress: { pass_index: passIndex, submitted_bytes: total, total_bytes: total } });
    emit({ PassComplete: passIndex });

    const pos = enabledIndices.indexOf(passIndex);
    const isLast = pos === enabledIndices.length - 1;
    if (isLast) {
      deviceState = "Idle";
      emit("JobComplete");
      emit({ StateChanged: "Idle" });
    } else {
      const next = enabledIndices[pos + 1];
      deviceState = { WaitingForColorSwap: { job_id: jobId, next_pass_index: next } };
      emit({ StateChanged: deviceState });
    }
  }

  Object.assign(commands, {
    list_devices: () => devices,
    connect_device: (a) => {
      const info = a.info as DeviceInfo;
      connected = info;
      deviceState = "Idle";
      return null;
    },
    disconnect_device: () => {
      connected = null;
      deviceState = "Disconnected";
      return null;
    },
    get_device_state: () => deviceState,
    plan_cut: () => planFromDoc(),
    cut: (a) => {
      const request = a.request as { device_instance_id: string; doc_revision: string; passes: { color: number | null; enabled: boolean }[] };
      if (!connected) throw ipcError("not_connected", "no device connected");
      if (connected.instance_id !== request.device_instance_id) {
        throw ipcError("device_mismatch", "connected device changed since planning");
      }
      const plan = planFromDoc();
      if (plan.doc_revision !== request.doc_revision) {
        throw ipcError("stale_plan", "document changed since the cut was planned; replan");
      }
      if (doc.machine && doc.machine.id !== connected.machine_id) {
        throw ipcError("machine_mismatch", "document is set up for a different machine");
      }
      planPasses = request.passes;
      jobId = nextJobId++;
      const enabledIndices = planPasses.map((p, i) => (p.enabled ? i : -1)).filter((i) => i >= 0);
      if (enabledIndices.length === 0) throw ipcError("nothing_to_cut", "no enabled passes");
      runPass(enabledIndices[0], enabledIndices);
      return jobId;
    },
    cancel_cut: () => {
      deviceState = "Idle";
      emit({ StateChanged: "Idle" });
      return null;
    },
    resume_cut: () => {
      const s = deviceState as { WaitingForColorSwap?: { next_pass_index: number } };
      const nextIndex = s.WaitingForColorSwap?.next_pass_index;
      if (nextIndex === undefined) throw ipcError("device_error", "not waiting for color swap");
      const enabledIndices = planPasses.map((p, i) => (p.enabled ? i : -1)).filter((i) => i >= 0);
      runPass(nextIndex, enabledIndices);
      return null;
    },
    confirm_pass_done: () => {
      deviceState = "Idle";
      emit({ StateChanged: "Idle" });
      return null;
    },
    list_presets: () => [],
    save_preset: () => null,
    delete_preset: () => null,
  } as Record<string, (args: Record<string, unknown>) => unknown>);

  (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    invoke: (cmd: string, args: Record<string, unknown> = {}) => {
      if (cmd === "plugin:dialog|save" || cmd === "plugin:dialog|open") {
        return Promise.resolve("/mock/cuthulhu-project.cut");
      }
      if (cmd === "plugin:event|listen") {
        const id = args.handler as number;
        const event = args.event as string;
        const ids = eventNameToIds.get(event) ?? [];
        ids.push(id);
        eventNameToIds.set(event, ids);
        return Promise.resolve(id);
      }
      if (cmd === "plugin:event|unlisten") {
        for (const ids of eventNameToIds.values()) {
          const i = ids.indexOf(args.eventId as number);
          if (i >= 0) ids.splice(i, 1);
        }
        return Promise.resolve(null);
      }
      const fn = commands[cmd];
      if (!fn) return Promise.reject(new Error(`unmocked command: ${cmd}`));
      try {
        return Promise.resolve(fn(args));
      } catch (e) {
        return Promise.reject(e instanceof Error ? e.message : e);
      }
    },
    transformCallback: (callback: (e: unknown) => void) => {
      const id = nextCallbackId++;
      callbacksById.set(id, callback);
      return id;
    },
  };
  // @tauri-apps/api/event's unlisten() path touches this directly; stub it so a
  // listener cleanup (e.g. on unmount) doesn't throw.
  (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: () => {},
  };
}

test("new doc → add rect → save → reload keeps the rect", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.getByRole("button", { name: "Rectangle" }).click();
  await page.mouse.click(400, 300);
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
  await page.getByRole("button", { name: "Save" }).click();

  // Discriminating step: delete the rect after Save so Reload can only pass by genuinely
  // restoring the saved copy, not by leaving live state untouched (a no-op load_project
  // would otherwise pass the final assertion below for free).
  await page.getByTestId("layer-row").click();
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(page.getByTestId("layer-row")).toHaveCount(0);

  await page.getByRole("button", { name: "Reload" }).click();
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
});

test("two-color doc cuts through swap and resume", async ({ page }) => {
  // Two differently-stroked rects are seeded synchronously inside the mock (no stroke
  // picker exists in the UI) so App.tsx's initial snapshot() already sees them.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: false }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();

  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByText(/complete/i)).toBeVisible();

  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
