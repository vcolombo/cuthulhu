// SPDX-License-Identifier: GPL-3.0-or-later
import { test, expect } from "@playwright/test";

// Minimal in-memory fake Tauri backend. Runs inside the page (via addInitScript, so it
// can't close over anything outside itself) and mirrors the JSON shape produced by
// crates/document's Document::snapshot_json() — see App.tsx's DocSnapshot/buildScene,
// which is what actually parses this on the JS side.
function installMockTauri(opts?: { seedTwoColorRects?: boolean; failImagePreview?: boolean; dropTraceControl?: string; seedBusyHost?: boolean; seedRemoteConnected?: boolean; slowList?: boolean; failList?: boolean; noFonts?: boolean }) {
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

  const unimplemented = (cmd: string): never => {
    throw new Error(`${cmd}: mocked command the e2e fake does not perform; implement it here to test it`);
  };

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    new_doc: () => {
      doc = freshDoc();
      return JSON.stringify(doc);
    },
    snapshot: () => JSON.stringify(doc),
    add_primitive: (a) => {
      const id = nextId++;
      const style = a.stroke !== undefined ? { stroke: a.stroke as number | null, fill: null } : DEFAULT_STYLE;
      doc.nodes[id] = { id, kind: { Shape: a.kind }, transform: [1, 0, 0, 1, 0, 0], style, children: [] };
      doc.nodes[a.parent as number].children.push(id);
      return {};
    },
    add_text: (a) => {
      // The real command rejects on the backend's terms; the fake can at least refuse a
      // caller that stopped forwarding the arguments, so the picker test proves the
      // selected family actually crosses the IPC boundary.
      if (typeof a.family !== "string" || a.family.length === 0) throw new Error("add_text: missing family");
      if (typeof a.sizeMm !== "number" || typeof a.text !== "string") throw new Error("add_text: missing sizeMm/text");
      const id = nextId++;
      doc.nodes[id] = { id, kind: { Shape: { Path: { d: "" } } }, transform: [1, 0, 0, 1, 0, 0], style: DEFAULT_STYLE, children: [] };
      doc.nodes[a.parent as number].children.push(id);
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
      return {};
    },
    // Four commands the fake has never performed. They used to answer "ok" while leaving
    // `doc` untouched, which is the false green this file exists to avoid: each one edits
    // the document in the real backend, so a plan made before it goes stale and `plan_cut`
    // refuses the cut. Refuse here too, loudly, rather than silently permitting one.
    reorder: () => unimplemented("reorder"),
    undo: () => unimplemented("undo"),
    redo: () => unimplemented("redo"),
    boolean_op: () => unimplemented("boolean_op"),
    import_svg: (a) => {
      const id = nextId++;
      doc.nodes[id] = { id, kind: { Shape: { Path: { d: "" } } }, transform: [1, 0, 0, 1, 0, 0], style: DEFAULT_STYLE, children: [] };
      doc.nodes[a.parent as number].children.push(id);
      return [{}, []];
    },
    save_project: () => {
      saved = JSON.parse(JSON.stringify(doc));
      return null;
    },
    load_project: () => {
      if (saved) doc = JSON.parse(JSON.stringify(saved));
      return JSON.stringify(doc);
    },
    set_machine: (a) => {
      const m = machines.find((p) => p.id === a.machineId);
      if (!m) throw new Error("unknown machine");
      doc.machine = m;
      doc.artboard = { x: 0, y: 0, w: m.width_mm, h: m.height_mm };
      return null;
    },
    list_machines: () => machines,
    // A fixture, not a claim: the real list comes from geometry::list_font_families and is
    // whatever the OS has installed. This exists only so the dialog has options to render.
    list_fonts: () => (opts?.noFonts ? [] : ["Arial", "Comic Sans MS", "Times New Roman"]),
    trace_image: () => ({
      svg: '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><path d="M0 0 L10 0 L10 10 L0 10 Z" fill="#000000"/></svg>',
      pathCount: 1, widthPx: 10, heightPx: 10, downscaled: false,
    }),
    // A fixture, not a claim: the real table lives in trace::CONTROLS and is what ships. This
    // exists only so the dialog has sliders to render.
    trace_controls: () => ({
      controls: [
        { name: "speckle", label: "Ignore speckles", help: "", min: 0, max: 16, step: 1, default: 4, colorOnly: false },
        { name: "smoothing", label: "Smoothing", help: "", min: 0, max: 180, step: 1, default: 60, colorOnly: false },
        { name: "detail", label: "Detail", help: "", min: 3.5, max: 10, step: 0.5, default: 9.5, colorOnly: false },
        { name: "colors", label: "Colors", help: "", min: 1, max: 8, step: 1, default: 6, colorOnly: true },
      ].filter((c) => c.name !== opts?.dropTraceControl),
      defaultMode: "binary",
      maxDim: 2048,
    }),
    load_image_preview: () => {
      if (opts?.failImagePreview) throw new Error("could not read image: broken thumbnail");
      return "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    },
  };

  // --- device / cut / preset mock: mirrors apps/desktop/src/device.rs's validation
  // order and driver-core::manager's DeviceEvent shape closely enough to drive the cut
  // dialog through a real state machine. The statuses below are transcribed from
  // driver-core/src/status.rs's status_of table — phase and the four action booleans
  // per internal state — so a frontend that re-derives permissions has nothing to
  // re-derive them from. ---

  type DeviceInfo = { instance_id: string; machine_id: string; transport: unknown; candidate: boolean; host: string | null };
  type Actions = { cut: boolean; cancel: boolean; resume: boolean; confirm: boolean };
  type CutStatus = {
    phase: string;
    ended: string | null;
    actions: Actions;
    pass: { index: number; total: number } | null;
    sent: { sent: number; total: number } | null;
    error: unknown;
  };
  type DeviceEvent = { job_id: number; kind: unknown; status: CutStatus };

  const NO_ACTIONS: Actions = { cut: false, cancel: false, resume: false, confirm: false };
  const statusOf = (phase: string, actions: Partial<Actions> = {}, rest: Partial<CutStatus> = {}): CutStatus => ({
    phase,
    ended: null,
    actions: { ...NO_ACTIONS, ...actions },
    pass: null,
    sent: null,
    error: null,
    ...rest,
  });
  const DISCONNECTED = statusOf("Disconnected");
  const CONNECTING = statusOf("Connecting");
  // `Idle` alone is a device that has cut nothing; a job that ran to the end rests on
  // the same phase and says so through `ended`, which is the whole of the difference.
  const IDLE = statusOf("Idle", { cut: true });
  const COMPLETED = statusOf("Idle", { cut: true }, { ended: "Completed" });

  const devices: DeviceInfo[] = [
    { instance_id: "usb:mock", machine_id: "cameo5", transport: { Usb: { locator: "mock" } }, candidate: false, host: null },
    { instance_id: "serial:/dev/mock0", machine_id: "puma", transport: { Serial: { path: "/dev/mock0", baud: 9600 } }, candidate: true, host: null },
  ];

  // A paired host that cannot be reached, with a cutter on it. Both halves matter: the row has
  // to stay listed with its reason (#42), and `forget_host` has to be refusable while it does.
  let hosts: { id: string; name: string; address: string; unreachable: string | null }[] = [];
  if (opts?.seedBusyHost) {
    hosts = [{ id: "host-1", name: "Workshop Pi", address: "pi.local:7878",
               unreachable: "the host could not be reached (timed out)" }];
    devices.push({ instance_id: "usb:pi:A", machine_id: "cameo5",
                   transport: { Usb: { locator: "pi" } }, candidate: false, host: "host-1" });
  }

  // The host a pairing reaches. One fingerprint, one token it accepts, one cutter behind it —
  // enough for the dialog's pairing flow to be driven end to end rather than stubbed out.
  const HOST_FINGERPRINT = "AB:CD:EF:01:23:45";
  const HOST_TOKEN = "correct-horse";
  const PAIRED_CUTTER: DeviceInfo = {
    instance_id: "usb:pi:B", machine_id: "cameo5",
    transport: { Usb: { locator: "pi" } }, candidate: false, host: "host-2",
  };

  // A reachable Cut Host with a cutter aimed at and stuck: cancelled, with nothing the operator
  // may do next. That is the state the Reconnect control exists for, and nothing in this suite
  // exercised a *remote* connected cutter at all — so the one line joining `connectedControl`'s
  // answer to `reconnect_device` was covered by neither half's tests (#123).
  const REMOTE_CUTTER: DeviceInfo = {
    instance_id: "usb:pi:C", machine_id: "cameo5",
    transport: { Usb: { locator: "pi" } }, candidate: false, host: "host-3",
  };
  if (opts?.seedRemoteConnected) {
    hosts.push({ id: "host-3", name: "Bench Pi", address: "bench.local:7878", unreachable: null });
    devices.push(REMOTE_CUTTER);
  }

  let connected: DeviceInfo | null = opts?.seedRemoteConnected ? REMOTE_CUTTER : null;
  // A cancel whose stop nothing confirmed: the Job is over, and `driver-core` still refuses a cut
  // until the transport is re-opened. `Idle` on both sides of the reconnect, which is exactly why
  // the control is derived from `actions` and not from the phase.
  let status: CutStatus = opts?.seedRemoteConnected
    ? { phase: "Idle", ended: "Cancelled", actions: { ...NO_ACTIONS }, pass: null, sent: null, error: null }
    : DISCONNECTED;
  let deviceStateCalls = 0;
  let listDeviceCalls = 0;
  let nextJobId = 1;
  let jobId: number | null = null;
  let planPasses: { color: number | null; enabled: boolean }[] = [];
  let failNextResume = false;
  let failNextCut = false;
  // Parked responses for the reorder/replan race, released from the test in the order it
  // wants to prove. Exposed on `window` rather than driven by timers: the defect is about
  // which reply lands last, and a sleep that guesses that is a flaky test, not a proof.
  // Armed by the test rather than by a call count — StrictMode plans twice on mount, so
  // "hold from the second call" holds the dialog's own opening plan and it never gets rows.
  let holding = false;
  const heldPlans: (() => void)[] = [];
  const heldTravel: (() => void)[] = [];
  const release = (queue: (() => void)[]) => {
    queue.splice(0).forEach((f) => f());
    // One macrotask, so the settled promises' handlers have run by the time the test's
    // `evaluate` resolves and it can assert on what they did (or did not) change.
    return new Promise((r) => setTimeout(r, 0));
  };
  Object.assign(window, {
    __armHold: () => { holding = true; },
    __releasePlans: () => release(heldPlans),
    __releaseTravel: () => release(heldTravel),
  });

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
    // starts is all-null on purpose: the fake carries no geometry to flatten, and null
    // is the real backend's no-outline case — so e2e renders exercise the preview's
    // bounds-corner badge fallback rather than a fixture pretending to be a blade path.
    const passes = [...byColor.values()].map((p) => ({
      color: p.color,
      shape_count: p.node_ids.length,
      node_ids: p.node_ids,
      starts: p.node_ids.map(() => null),
    }));
    // The snapshot itself is the revision, mirroring cutplan::doc_revision hashing
    // snapshot_json: a doc edited back to a previous state is not stale. A counter
    // bumped per command diverges on that, and silently goes stale-blind for any
    // command that mutates `doc` and forgets to bump (commit_transform did).
    return { passes, skipped_no_stroke: skipped, doc_revision: JSON.stringify(doc), travel: [] as [number, number, number, number][] };
  }

  // Mirrors @tauri-apps/api/event's listen()/transformCallback() plumbing: listen()
  // calls transformCallback(handler) to get a numeric id (stored in callbacksById),
  // then invoke("plugin:event|listen", {event, handler: id}) associates that id with
  // an event name (eventNameToIds). Emitting calls the stored callback directly, like
  // the real event bridge thread does via window["_" + id](payload).
  const callbacksById = new Map<number, (e: unknown) => void>();
  const eventNameToIds = new Map<string, number[]>();
  let nextCallbackId = 1;

  // Mirrors Reporter::emit: the event carries the status that held when it was sent, so
  // a listener renders from what it received rather than polling for a newer value.
  function emit(kind: unknown) {
    const ev: DeviceEvent = { job_id: jobId ?? 0, kind, status };
    for (const id of eventNameToIds.get("device-event") ?? []) {
      callbacksById.get(id)?.({ event: "device-event", id, payload: ev });
    }
  }

  // Drives the scripted pass sequence for one pass, then either pauses at
  // WaitingForColorSwap (more enabled passes remain) or completes the job — matching
  // execute_cut's documented behavior of blocking until the next pause point.
  //
  // The initial Transmitting(0 bytes) StateChanged fires synchronously (matches real
  // execute_cut, which enters Transmitting immediately) but everything after it is
  // deferred a tick: in production, event delivery crosses real async Tauri IPC (worker
  // thread -> event bridge -> window.emit/listen), giving React's setJobId(null)/
  // jobIdRef sync effect time to commit before any event arrives. This mock's invoke()
  // used to run every command fully synchronously in the same call stack as the click
  // handler, so a second cut's events could arrive before React had committed the new
  // jobId — a mock-fidelity gap, not a production bug. The setTimeout also makes
  // Transmitting observable to Playwright's polling assertions instead of being
  // superseded within the same synchronous burst.
  function runPass(passIndex: number, enabledIndices: number[]) {
    const total = 100;
    const sending = (sent: number) =>
      statusOf("Sending", { cancel: true }, { pass: { index: passIndex, total: enabledIndices.length }, sent: { sent, total } });
    status = sending(0);
    emit("StateChanged");
    setTimeout(() => {
      status = sending(total);
      emit({ Progress: { pass_index: passIndex, submitted_bytes: total, total_bytes: total } });
      emit({ PassComplete: passIndex });

      const pos = enabledIndices.indexOf(passIndex);
      const isLast = pos === enabledIndices.length - 1;
      if (isLast) {
        // finish_pass emits JobComplete *before* the state becomes Idle, so this event's
        // status still reads Sending. A frontend that treats the JobComplete kind as
        // "finished" would show a completed job as still cutting; only the Idle status
        // below says the job is over.
        emit("JobComplete");
        status = COMPLETED;
        emit("StateChanged");
        // Production releases the job id once the job is over: all later lifecycle
        // events (reconnects, state refreshes) carry NO_JOB=0. Mirror that, or the
        // mock keeps stamping finished-job ids on lifecycle events production
        // would never stamp.
        jobId = null;
      } else {
        const next = enabledIndices[pos + 1];
        status = statusOf(
          "AwaitingColorSwap",
          { cancel: true, resume: true },
          { pass: { index: next, total: enabledIndices.length } },
        );
        emit("StateChanged");
      }
    }, 50);
  }

  Object.assign(commands, {
    // A copy, as a Rust `Vec<DeviceInfo>` crossing the IPC boundary is. Handing out the live
    // array let a later mutation here reach into React's own state and repair a list the
    // frontend had not re-read — a fake that quietly fixes the frontend's bugs for it.
    //
    // Slow from the second call on when asked, so the first read (the one that puts a host in
    // the list at all) does not itself eat the window a test is trying to observe.
    list_devices: () => {
      if (opts?.failList) throw ipcError("device_error", "the device list could not be read");
      const slow = opts?.slowList && listDeviceCalls++ > 0;
      return slow ? new Promise((r) => setTimeout(() => r([...devices]), 3000)) : [...devices];
    },
    connect_device: (a) => {
      const info = a.info as DeviceInfo;
      connected = info;
      // Production emits connect lifecycle StateChanged events with NO_JOB=0 —
      // emitting them here (instead of silently mutating the status) is what lets
      // these tests catch a frontend that filters lifecycle events out.
      status = CONNECTING;
      emit("StateChanged");
      status = IDLE;
      emit("StateChanged");
      return null;
    },
    disconnect_device: () => {
      connected = null;
      status = DISCONNECTED;
      emit("StateChanged");
      return null;
    },
    // Keeps the aim, unlike `disconnect_device`: the cutter is still there and still aimed at, it
    // has simply had its transport re-opened. On a Cut Host this is the host's own verb — this
    // desktop never opened that transport, so a disconnect here would leave it exactly as stuck.
    reconnect_device: () => {
      if (!connected) throw ipcError("device_error", "no device is connected");
      status = IDLE;
      emit("StateChanged");
      return null;
    },
    get_device_state: () => {
      // Counted because the dialog's poll is only observable as a call rate: what has to be
      // proven is that it stops, and a stopped interval leaves no other trace.
      deviceStateCalls++;
      return status;
    },
    get_connected_device: () => connected,
    plan_cut: () => {
      // Answered from the document as it is *now*, like the real command, then parked if
      // the test has armed the hold: which of a replan and an older reorder settles first
      // is the whole subject of the race test, and a timing race cannot state it.
      const plan = planFromDoc();
      if (!holding) return plan;
      return new Promise((resolve) => heldPlans.push(() => resolve(plan)));
    },
    // Mirrors device::travel_for_order's contract, not its geometry: the same stale-plan
    // refusal, then synthetic segments (one per adjacent pair, x encoding the position in
    // the order). Received orders are recorded on `window.__travelOrders` so a test can
    // assert what the dialog asked for — travel itself lands on a canvas Playwright
    // cannot read.
    travel_for_order: (a) => {
      const order = a.order as (number | null)[];
      (window as unknown as { __travelOrders: (number | null)[][] }).__travelOrders ??= [];
      (window as unknown as { __travelOrders: (number | null)[][] }).__travelOrders.push(order);
      // Decided against the document at call time, like the real command — a request
      // issued before a replan is stale even if it settles after one.
      const stale = planFromDoc().doc_revision !== a.docRevision;
      const settle = () => {
        if (stale) throw ipcError("stale_plan", "document changed since the cut was planned; replan");
        return order.slice(1).map((_, i) => [i, 0, i + 1, 0] as [number, number, number, number]);
      };
      if (!holding) return settle();
      return new Promise((resolve, reject) => heldTravel.push(() => {
        try { resolve(settle()); } catch (e) { reject(e); }
      }));
    },
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
      if (failNextCut) {
        // The opening write dies: Sending and then Failed both go out in this same
        // synchronous burst, so the only status the frontend ever commits is the failed
        // one — the mid-flight status it faulted from never reaches a render.
        failNextCut = false;
        const id = jobId;
        status = statusOf("Sending", { cancel: true }, { pass: { index: enabledIndices[0], total: enabledIndices.length }, sent: { sent: 0, total: 100 } });
        emit("StateChanged");
        emit({ Failed: "Timeout" });
        status = statusOf("Failed", {}, { error: "Timeout" });
        emit("StateChanged");
        jobId = null;
        return { job_id: id, duplicate: false };
      }
      runPass(enabledIndices[0], enabledIndices);
      // `duplicate` is the Cut Host's own answer — it is the only party that knows whether it had
      // already accepted this dispatch id. This fake stands in for a local cutter, which has no
      // dedupe to be caught by, so it is always false here.
      return { job_id: jobId, duplicate: false };
    },
    cancel_cut: () => {
      // Mirrors driver-core::manager: cancel is unconditional and lands on the
      // Cancelled resting state — nothing is happening, so the phase is Idle, but
      // `ended` names the cancel. Cut is legal again only because this fake stands in
      // for a pollable cutter whose stop was confirmed; a Puma's never is, and
      // status.rs's Cancelled arm then withholds `actions.cut`.
      const sent = status.sent?.sent ?? 0;
      const pass = status.pass;
      // CancelRequested then Stopping — two distinct internal states that both report
      // phase Cancelling with *no* actions at all. The dialog has to survive losing
      // every button for a moment, so the mock must not skip them.
      status = statusOf("Cancelling");
      emit("StateChanged");
      status = statusOf("Cancelling");
      emit("StateChanged");
      // The worker only rests on Cancelled once it has woken and stopped, so the
      // resting state lands a tick later, as it does in production.
      setTimeout(() => {
        status = statusOf("Idle", { cut: true }, { ended: "Cancelled", pass, sent: { sent, total: sent } });
        emit("StateChanged");
        jobId = null; // job over — later lifecycle events are NO_JOB=0, as in production
      }, 50);
      return null;
    },
    resume_cut: () => {
      if (status.phase !== "AwaitingColorSwap") throw ipcError("device_error", "not waiting for color swap");
      const nextIndex = status.pass?.index ?? 0;
      if (failNextResume) {
        // Async failure, as in production: resume_cut returns Ok and the failure
        // arrives via the event stream (Failed carries the job's id, then the
        // device rests in Error). Job over — id released so later lifecycle
        // events go out as NO_JOB=0.
        failNextResume = false;
        setTimeout(() => {
          emit({ Failed: "Timeout" });
          status = statusOf("Failed", {}, { error: "Timeout" });
          emit("StateChanged");
          jobId = null;
        }, 50);
        return null;
      }
      const enabledIndices = planPasses.map((p, i) => (p.enabled ? i : -1)).filter((i) => i >= 0);
      runPass(nextIndex, enabledIndices);
      return null;
    },
    // Test hook (no production counterpart): arms a one-shot failure for the next
    // resume_cut so tests can drive the failed-job → reconnect recovery path.
    __test_fail_next_resume: () => {
      failNextResume = true;
      return null;
    },
    // Same, for a fault during the first pass of the next cut.
    __test_fail_next_cut: () => {
      failNextCut = true;
      return null;
    },
    // Test hook (no production counterpart): how many times the dialog has asked for a status.
    __test_poll_count: () => deviceStateCalls,
    confirm_pass_done: () => {
      status = COMPLETED;
      emit("StateChanged");
      return null;
    },
    list_presets: () => [],
    // Deliberately one constant, not a per-machine table: that mapping is pinned in
    // Rust by each Driver's own caps test, and restating it here would recreate the
    // copy this change removed — in a file nobody thinks of as production code.
    machine_caps: () => ({ supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false }),
    // Empty unless a test asks for a host, so existing assertions (device list, connect flow)
    // are unaffected.
    list_hosts: () => hosts,
    // Sends no token, so it answers whatever the address is — mirroring a TLS handshake that
    // has vouched for nothing yet.
    probe_host: () => HOST_FINGERPRINT,
    // Whether this address is already paired, asked between the probe and the confirm. Mirrors
    // `DeviceManagerHandle::existing_pairing`: matched on the address as typed.
    existing_pairing: (a) => {
      const already = hosts.find((h) => h.address === a.address);
      if (!already) return null;
      return { id: already.id, name: already.name, sameFingerprint: a.fingerprint === HOST_FINGERPRINT };
    },
    // Refuses the token rather than the address: a host that answers but does not accept this
    // token is the failure the pairing flow exists to catch before anything is saved.
    test_host: (a) => {
      if (a.fingerprint !== HOST_FINGERPRINT) throw ipcError("host_unreachable", "the host's fingerprint changed");
      if (a.token !== HOST_TOKEN) throw ipcError("host_unreachable", "the token was refused");
      return [PAIRED_CUTTER];
    },
    pair_host: (a) => {
      const host = { id: "host-2", name: a.name as string, address: a.address as string, unreachable: null };
      hosts.push(host);
      // The cutter is only in `list_devices` once the host is paired, which is what the dialog
      // re-reads for: the Test listed it to prove the token, not to populate the device list.
      devices.push(PAIRED_CUTTER);
      return host;
    },
    // Mirrors `DeviceManagerHandle::forget`: a host that cannot be asked whether it is cutting
    // refuses like one that answered "busy" — the Pi keeps cutting when the network drops, and
    // the desktop must keep the row rather than discard the token for a Job it could no longer
    // cancel. Distinct code, because only this one can be forced past.
    forget_host: (a) => {
      if (!a.force && hosts.some((h) => h.id === a.id && h.unreachable !== null))
        throw ipcError(
          "host_unconfirmed",
          "this Cut Host could not be asked whether it is cutting (timed out); if it is, forgetting it discards the only way to stop it",
        );
      hosts = hosts.filter((h) => h.id !== a.id);
      // Its cutters go with it, as they do in Rust: `list_devices` reaches a host through the
      // pairing that was just discarded, so it cannot still be reporting what is attached to it.
      for (let i = devices.length - 1; i >= 0; i--) if (devices[i].host === a.id) devices.splice(i, 1);
      return null;
    },
    save_preset: () => null,
    delete_preset: () => null,
    // The picker now lives in Rust so the backend, not the caller, decides what is readable.
    pick_image: () => "/tmp/fake.png",
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
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();

  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByText(/complete/i)).toBeVisible();

  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

// The plan is made from the document, so an edit after planning must refuse the cut —
// cutplan::plan_cut's stale-plan rule. `commit_transform` is the discriminating edit:
// it changes geometry without adding or removing a node, so a revision that tracks
// commands rather than the document itself cuts stale geometry and still passes.
test("a doc edited after planning refuses the cut until replan", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  // Reaches past the UI on purpose: the canvas drag that issues this command is behind
  // the open dialog, and the backend contract under test is the same either way.
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
      "commit_transform",
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
    ),
  );

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Document changed since this plan was made.")).toBeVisible();

  await page.getByRole("button", { name: "Replan" }).click();
  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();
});

test("reordering passes asks the backend for travel in the new order", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  // Seeded first-seen order is red (0xff0000ff) then green (0x00ff00ff).
  const chip = (i: number) => page.getByTestId("cut-pass-row").nth(i).locator("span").first();
  await expect(chip(0)).toHaveCSS("background-color", "rgb(255, 0, 0)");

  await page.getByRole("button", { name: "Down" }).first().click();
  await expect(chip(0)).toHaveCSS("background-color", "rgb(0, 255, 0)");

  // The wire is the contract under test: the replan request names the swapped order.
  const orders = await page.evaluate(
    () => (window as unknown as { __travelOrders?: (number | null)[][] }).__travelOrders,
  );
  expect(orders).toEqual([[0x00ff00ff, 0xff0000ff]]);
});

test("reordering after a doc edit surfaces the stale plan instead of stale travel", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  // Same off-screen edit as the stale-cut test above: the dialog covers the canvas.
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
      "commit_transform",
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
    ),
  );

  await page.getByRole("button", { name: "Down" }).first().click();
  await expect(page.getByText("Document changed since this plan was made.")).toBeVisible();
});

// The order these two settle in is the whole defect: a reorder issued before Replan carries
// the old revision, so it is refused — and that refusal arriving *after* the fresh plan
// installed used to re-raise the banner the replan had just cleared, telling the operator a
// document they had only now replanned was stale again.
test("a reorder refused for the old revision does not re-mark a freshly replanned document", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
      "commit_transform",
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
    ),
  );
  const banner = page.getByText("Document changed since this plan was made.");
  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(banner).toBeVisible();

  // Replan is in flight (held) when the pass moves, so the move is sent with the revision
  // the fresh plan is about to replace.
  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByRole("button", { name: "Replan" }).click();
  await page.getByRole("button", { name: "Down" }).first().click();

  await page.evaluate(() => (window as unknown as { __releasePlans: () => Promise<unknown> }).__releasePlans());
  await expect(banner).toHaveCount(0);

  await page.evaluate(() => (window as unknown as { __releaseTravel: () => Promise<unknown> }).__releaseTravel());
  await expect(banner).toHaveCount(0);
});

// The one test that drives the Cut Host surface end to end. It is here rather than in a unit
// test because the parts it checks are exactly the ones a unit test cannot: that the grouped
// list reaches the dialog at all, and that a refusal from the backend leaves the row on screen.
test("an unreachable host keeps its cutters listed, and refusing to be forgotten keeps its row", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedBusyHost: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();

  // Listed with the reason, not hidden — a cutter that vanishes looks exactly like one that was
  // never paired (#42).
  await expect(page.getByText("Workshop Pi")).toBeVisible();
  await expect(page.getByText("the host could not be reached (timed out)")).toBeVisible();
  // Its cutter is still a row: two local cutters plus this one. None has been polled, so every
  // badge says so rather than offering a cut for a status nobody has asked for.
  await expect(page.getByText("Unknown")).toHaveCount(3);

  await page.getByRole("button", { name: "Forget Workshop Pi" }).click();
  // The Rust side's own words, and the row still there (#94).
  await expect(page.getByText("this Cut Host could not be asked whether it is cutting")).toBeVisible();
  await expect(page.getByText("Workshop Pi")).toBeVisible();

  // Only now is the force on screen, and it says what is being accepted rather than asking
  // whether the operator is sure.
  await expect(page.getByText(/A cut may still be running on this Cut Host/)).toBeVisible();
  await page.getByRole("button", { name: "Discard Workshop Pi anyway" }).click();
  // A Pi that is gone for good must not become unforgettable — the row and its cutter both go.
  await expect(page.getByRole("button", { name: /^Forget/ })).toHaveCount(0);
  expect(await page.getByText("Workshop Pi").count()).toBe(0);
});

// Reaching a Pi starts here, and nothing drove it end to end while the fake refused the three
// pairing commands. The order is the security property: the fingerprint reaches the operator
// before the token reaches the host, so a rejected fingerprint has told the far end nothing.
test("pairing a Cut Host shows its fingerprint first, then lists it with its cutters", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: /Add a Cut Host/ }).click();

  await page.getByLabel("Address").fill("pi.local:7878");
  await page.getByLabel("Token").fill("correct-horse");
  await page.getByLabel("Name (optional)").fill("Workshop Pi");
  await page.getByRole("button", { name: "Pair", exact: true }).click();

  await expect(page.getByText("AB:CD:EF:01:23:45")).toBeVisible();
  await page.getByRole("button", { name: /It matches/ }).click();

  // The host is in the device list with the cutter the Test proved, and it can be forgotten —
  // the whole affordance, from an empty list to a usable remote cutter.
  await expect(page.getByRole("button", { name: "Forget Workshop Pi" })).toBeVisible();
  await expect(page.getByText("pi.local:7878")).toBeVisible();
});

// A wrong token must not leave a host saved. `pair_host` is what persists, and it is never
// reached: the row would otherwise have to be forgotten by hand before a retry could work.
test("a refused token pairs nothing and says so in the host's own words", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: /Add a Cut Host/ }).click();

  await page.getByLabel("Address").fill("pi.local:7878");
  await page.getByLabel("Token").fill("wrong");
  await page.getByRole("button", { name: "Pair", exact: true }).click();
  await page.getByRole("button", { name: /It matches/ }).click();

  await expect(page.getByRole("alert")).toHaveText("the token was refused");
  await expect(page.getByRole("button", { name: /^Forget/ })).toHaveCount(0);
});

// A host that is forgotten has to leave with its cutters. Dropping the row alone leaves them
// naming a host nobody is paired with, which is the one thing `groupDevices` refuses to hide:
// the row comes back as a raw host id under "this Cut Host is not paired with this computer".
// The count is taken once, not awaited — the wrong row is the state right after the click, and
// an assertion that retries would sit there until something else repaired it.
test("forgetting a host takes its cutters with it, instead of renaming the row to a raw id", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: /Add a Cut Host/ }).click();
  await page.getByLabel("Address").fill("pi.local:7878");
  await page.getByLabel("Token").fill("correct-horse");
  await page.getByLabel("Name (optional)").fill("Workshop Pi");
  await page.getByRole("button", { name: "Pair", exact: true }).click();
  await page.getByRole("button", { name: /It matches/ }).click();
  await expect(page.getByRole("button", { name: "Forget Workshop Pi" })).toBeVisible();

  await page.getByRole("button", { name: "Forget Workshop Pi" }).click();
  await expect(page.getByRole("button", { name: /^Forget/ })).toHaveCount(0);
  expect(await page.getByText("this Cut Host is not paired with this computer").count()).toBe(0);
  expect(await page.getByText("host-2").count()).toBe(0);
});

// The recovery path had both halves tested and the join between them untested: `connectedControl`
// is unit-tested for which control to show, `Host::reconnect` is tested against a real loopback
// Cut Host, and the line that turns the first into the second — `verb === "reconnect" ? ... : ...`
// — was covered by nothing, because the fake's connected device was always local (#123).
//
// A remote fixture rather than a `reconnect_device` handler bolted onto a local one: a fake that
// can ship a wrong command name green is the shape of #85, and the honest version of this test
// needs a remote connected cutter anyway — nothing in this suite had one.
test("a stuck cutter on a Cut Host offers Reconnect, and reconnecting makes it cuttable again", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedRemoteConnected: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();

  // The cancel nobody could confirm: the badge asks for a person, and Start Cut is withheld.
  await expect(page.getByText("Bench Pi")).toBeVisible();
  await expect(page.getByText(/Cancelled — stop not confirmed/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeDisabled();

  // Reconnect, not Disconnect: this desktop never opened that transport, and dropping the aim
  // would leave the cutter exactly as stuck.
  await expect(page.getByRole("button", { name: "Disconnect usb:pi:C" })).toHaveCount(0);
  await page.getByRole("button", { name: "Reconnect usb:pi:C" }).click();

  await expect(page.getByText("Ready", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
});

// The teardown is the deliverable, not the interval. A leaked interval keeps a Cut Host
// connection warm forever, and the daemon caps concurrent clients at eight (#103) — a desktop
// that leaks one per dialog-open exhausts a Pi, which then refuses every new connection until
// it is restarted.
const pollCount = (page: import("@playwright/test").Page) => () =>
  page.evaluate(
    () =>
      (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
        "__test_poll_count",
      ) as Promise<number>,
  );

test("the dialog polls while it is open, and stops once it is closed", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedBusyHost: true });
  await page.goto("/");
  const polls = pollCount(page);

  await page.getByRole("button", { name: "Cut" }).click();
  const opened = await polls();
  await expect.poll(polls, { timeout: 5000 }).toBeGreaterThan(opened);

  await page.getByRole("button", { name: "Close" }).click();
  // A tick already in flight when the dialog closed still lands, so settle past one full period
  // before taking the reading that must not move.
  await page.waitForTimeout(1500);
  const closed = await polls();
  await page.waitForTimeout(2500);
  expect(await polls()).toBe(closed);
});

// The stale path, on the desktop that has no host: the local section's heading is normally
// suppressed, and the "last known" marker lives inside it, so a failed read showed as one
// unlabelled red line of Rust prose under "Device" — no heading, no marker, no banner.
test("a device list that cannot be read keeps its heading, so the failure has something to name", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, failList: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();

  await expect(page.getByText("This computer")).toBeVisible();
  await expect(page.getByText("last known")).toBeVisible();
  await expect(page.getByText("the device list could not be read")).toBeVisible();
});

// The Pi is optional, and a desktop without one has nothing polling can tell it: a local
// cutter's status is pushed. Before this branch `list_devices` ran once per dialog open, and it
// walks the USB bus and the serial ports — running it every second for a user who owns no host
// is a cost with no answer on the other end of it.
test("a desktop with no Cut Host paired does not poll at all", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.waitForTimeout(2500);
  expect(await pollCount(page)()).toBe(0);
});

// The other half of polling safely: an unreachable host can take seconds to answer, and a tick
// that waits its turn instead of being skipped builds a backlog that outlives whatever wedged it.
test("a tick whose last request is still in flight is skipped, not queued", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedBusyHost: true, slowList: true });
  await page.goto("/");
  const polls = pollCount(page);

  await page.getByRole("button", { name: "Cut" }).click();
  await page.waitForTimeout(4500);
  // Four ticks have fired; at three seconds an answer, at most two of them can have started
  // requests. Without the skip the count tracks the tick rate instead, and every extra request
  // is one more thing the host owes an answer to.
  const seen = await polls();
  expect(seen).toBeGreaterThanOrEqual(1);
  expect(seen).toBeLessThanOrEqual(2);
});

test("cancel mid-cut shows Cancelled and re-enables Start Cut", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByText("Cancelled", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
});

test("a cut that runs to the end reports completion, not a cancellation", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();

  // The backend says which ending it was, so the two endings cannot be confused: this
  // is the case the mock could not express while a finished cut only rested on `Idle`.
  await expect(page.getByText("Job complete")).toBeVisible();
  await expect(page.getByText("Cancelled", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
});

test("transmitting shows a Cancel button and progress so the GUI can cancel mid-cut", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText(/sending \d+ \/ \d+ bytes/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();

  // The tone reaches the row, so the cutting one is not the same flat grey as the cutter nobody
  // has polled. `deviceBadge`'s unit tests assert on `tone` alone; this is what makes them
  // describe the product instead of a field it threw away. The unpolled row is `unknown`, not
  // `attention` — nothing is wrong with it, and red is how this UI says something is.
  await expect(page.getByTestId("device-badge").first()).toHaveAttribute("data-tone", "busy");
  await expect(page.getByTestId("device-badge").nth(1)).toHaveAttribute("data-tone", "unknown");
});

test("second cut in the same dialog session also reaches waiting-for-swap", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByText(/complete/i)).toBeVisible();

  // Second cut in the same session: the dialog must take the new job's statuses as
  // they arrive, with no per-job bookkeeping left over from the finished first cut.
  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume" })).toBeVisible();
});

test("reopening the dialog after connect recovers the connected device", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByText("connected")).toBeVisible();

  await page.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Cut" }).click();

  // Without get_connected_device seeding this on mount, the reopened dialog's local
  // `connected` state comes back null even though the backend is still connected,
  // leaving Start Cut stuck disabled and the device row stuck on "Connect".
  await expect(page.getByText("connected")).toBeVisible();
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
});

// The exit from a cancel whose stop nothing confirmed. `driver-core` refuses both a cut and a
// connect from that state, so with no Disconnect in the dialog the operator's only way back to
// their own cutter is restarting the app — the workaround being "never cancel". This drives the
// control rather than the command behind it, because the control is what was missing.
test("a connected cutter offers a disconnect, and the row goes back to offering Connect", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByText("connected")).toBeVisible();

  await page.getByRole("button", { name: "Disconnect", exact: false }).first().click();
  await expect(page.getByText("connected")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Connect", exact: true }).first()).toBeVisible();
});

// Inverted deliberately. While the dialog latched its own outcome, "did the last cut
// finish?" was answered by how long the dialog had been mounted, so a reopened dialog
// had to show nothing — and this test asserted that. The outcome now comes from the
// device, so a reopened dialog reports what the device actually last did, for the same
// reason a freshly opened one does. The guard against a *false* completion is
// "disconnecting mid-pause" below: there the job never ended, and no banner appears.
test("a reopened dialog reports the ending the device actually last had", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByText("Job complete")).toBeVisible();

  await page.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByText("Job complete")).toBeVisible();
  await expect(page.getByText("Cancelled", { exact: true })).toHaveCount(0);
});

test("failed cut shows Cut failed and a reconnect recovers the device", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();

  // Arm a one-shot async resume failure, then resume: the Failed phase must show
  // "Cut failed" — never "Job complete", which a banner derived from "a job ended
  // and the device is Idle" produced for a failed job whose state cache lagged.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_resume"));
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByText("Cut failed")).toBeVisible();
  await expect(page.getByText("Job complete")).toHaveCount(0);

  // Recover by reconnecting (the other listed device). The connect lifecycle events
  // carry NO_JOB=0 and must still reach the dialog after a failed job — Start Cut
  // comes back only because the reconnect's Idle status was accepted.
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
});

// A fault whose mid-flight status never reaches a render: a banner that waits to be
// handed a "a cut was running" status first shows nothing at all here.
test("a cut that faults immediately still shows Cut failed", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_cut"));
  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Cut failed")).toBeVisible();
  await expect(page.getByText("Job complete")).toHaveCount(0);
});

// A completed banner belongs to the connection that cut it: a reconnect is a fresh
// device that has cut nothing, so the ending must go with the old connection. This is
// the reconnect half of what the dialog's latch used to get for free by remounting —
// the status has to clear it, and driver-core's lifecycle emit is what does.
test("a reconnect clears the completed banner from the previous connection", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByText("Job complete")).toBeVisible();

  // Driven through the IPC surface rather than the dialog's own Disconnect button: this is
  // about the device dropping out, not about the operator asking it to.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("disconnect_device"));
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
  await expect(page.getByText("Job complete")).toHaveCount(0);
});

// Losing the device mid-pause abandons the job. The reconnect reports Idle — the same
// phase a finished cut rests on — so a dialog that remembers "a cut was running" across
// the disconnect declares a job that never finished complete.
test("disconnecting mid-pause does not report the abandoned cut as complete", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();

  // Driven through the IPC surface rather than the dialog's own Disconnect button: this is
  // about the device dropping out, not about the operator asking it to.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("disconnect_device"));
  await expect(page.getByRole("button", { name: "Resume" })).toHaveCount(0);

  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
  await expect(page.getByText(/complete/i)).toHaveCount(0);
});

test("trace dialog: preview appears and insert adds paths", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.getByRole("button", { name: "Trace" }).click();
  await expect(page.getByRole("dialog", { name: "Trace image" })).toBeVisible();
  await expect(page.getByAltText("Traced preview")).toBeVisible();
  await expect(page.getByText("1 path")).toBeVisible();
  await page.getByRole("button", { name: "Insert" }).click();
  await expect(page.getByRole("dialog", { name: "Trace image" })).not.toBeVisible();
  // import_svg mock was invoked — it adds a node to doc, so the layer list reflects the
  // insert, same observable-effect assertion the "new doc → add rect" test above uses.
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
});

// `controlsFromSpecs` throws when the table omits a control, so that a dialog and a tracer
// disagreeing about what a trace takes is visible rather than papered over with an invented
// default. That only holds if the throw reaches the error state: it happens inside a `then`
// fulfillment handler, and a rejection handler passed to the *same* `then` does not catch it —
// the dialog would sit idle with no sliders, no error, and an unhandled rejection in the console.
test("trace dialog: a control table missing a control reports it instead of hanging", async ({ page }) => {
  await page.addInitScript(installMockTauri, { dropTraceControl: "detail" });
  await page.goto("/");
  await page.getByRole("button", { name: "Trace" }).click();
  const dialog = page.getByRole("dialog", { name: "Trace image" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/detail/)).toBeVisible();
  // Nothing traceable was ever configured, so Insert must not offer geometry.
  await expect(page.getByRole("button", { name: "Insert" })).toBeDisabled();
});

test("text dialog: picking a family and Insert adds a shape", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.getByRole("button", { name: "Text" }).click();
  const dialog = page.getByRole("dialog", { name: "Add text" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Font family").selectOption("Times New Roman");
  await page.getByRole("button", { name: "Insert" }).click();
  await expect(dialog).not.toBeVisible();
  // add_text mock was invoked — it adds a node to doc, same observable-effect assertion
  // the trace-insert test above uses.
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
});

test("text dialog: an empty font list says so and disables Insert", async ({ page }) => {
  await page.addInitScript(installMockTauri, { noFonts: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Text" }).click();
  const dialog = page.getByRole("dialog", { name: "Add text" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("No fonts were found on this system")).toBeVisible();
  await expect(page.getByRole("button", { name: "Insert" })).toBeDisabled();
});

// The traced pane covers the common case where a file fails to decode, because both commands
// fail together. It does not cover a thumbnail that fails on its own — re-encoding the preview
// can fail while the trace succeeds — and the design spec promises every error path surfaces
// rather than turning into an empty pane.
test("trace dialog: a failed source thumbnail surfaces instead of blanking", async ({ page }) => {
  await page.addInitScript(installMockTauri, { failImagePreview: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Trace" }).click();
  const dialog = page.getByRole("dialog", { name: "Trace image" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/broken thumbnail/)).toBeVisible();
  // The trace itself still succeeded, so the dialog stays usable.
  await expect(page.getByText("1 path")).toBeVisible();
  await expect(page.getByRole("button", { name: "Insert" })).toBeEnabled();
});
