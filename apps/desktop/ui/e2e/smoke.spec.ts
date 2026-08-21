// SPDX-License-Identifier: GPL-3.0-or-later
import { test, expect, type Page } from "@playwright/test";

// Minimal in-memory fake Tauri backend. Runs inside the page (via addInitScript, so it
// can't close over anything outside itself) and mirrors the JSON shape produced by
// crates/document's Document::snapshot_json() — see App.tsx's DocSnapshot/buildScene,
// which is what actually parses this on the JS side.
function installMockTauri(opts?: { seedTwoColorRects?: boolean; failImagePreview?: boolean; dropTraceControl?: string; seedBusyHost?: boolean; seedRemoteConnected?: boolean; slowList?: boolean; failList?: boolean; noFonts?: boolean; seedMachine?: boolean; seedUserPreset?: boolean; seedEmptyPresetAssignment?: boolean }) {
  type Style = { stroke: number | null; fill: number | null };
  type PresetAssignment = { state: "inherit" } | { state: "unassigned" } | { state: "preset"; id: string };
  type Node = { id: number; kind: unknown; transform: number[]; style: Style; children: number[]; cut_line_type: "Cut" | "NoCut"; material_preset: PresetAssignment };
  type Grouping = "Single" | "Color" | "Stroke" | "Fill" | "Preset";
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

  // Mirrors document::Style::default() — a freshly-added shape has an opaque black stroke.
  const DEFAULT_STYLE: Style = { stroke: 0x000000ff, fill: null };
  // Every Node below carries `cut_line_type: "Cut"`, the import default. It comes from the two
  // constructors, `Node::shape` and `Node::container` (crates/document/src/node.rs) — there is no
  // `CutLineType::default()` to mirror, and the planner reads the attribute rather than defaulting
  // it. That attribute, not the paint, is what makes a shape cuttable. Containers carry it inertly.

  let nextId = 1;
  const freshDoc = (): Doc => {
    const rootId = nextId++;
    return {
      nodes: { [rootId]: { id: rootId, kind: "Layer", transform: [1, 0, 0, 1, 0, 0], style: { stroke: null, fill: null }, children: [], cut_line_type: "Cut", material_preset: { state: "inherit" } } },
      root: rootId,
      artboard: { x: 0, y: 0, w: 330, h: 3000 },
      // Presets are machine-scoped, so a test that needs the Material control to offer anything
      // has to name a machine: App reads the list for the document's machine.
      machine: opts?.seedMachine
        ? { id: "cameo5", name: "Silhouette Cameo 5 Alpha", width_mm: 330, height_mm: 3000 }
        : null,
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
      cut_line_type: "Cut",
      material_preset: opts?.seedEmptyPresetAssignment
        ? { state: "preset", id: "" }
        : { state: "inherit" },
    };
    const greenId = nextId++;
    doc.nodes[greenId] = {
      id: greenId,
      kind: { Shape: { Rect: { x: 20, y: 0, w: 10, h: 10 } } },
      transform: [1, 0, 0, 1, 0, 0],
      style: { stroke: 0x00ff00ff, fill: null },
      children: [],
      cut_line_type: "Cut",
      material_preset: { state: "inherit" },
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
      // Same test-only override as `a.stroke`: no UI control sets cuttability at creation, so
      // this is the only way to seed a NoCut shape without going through set_cut_line_type.
      const cutLineType = a.cut_line_type !== undefined ? (a.cut_line_type as "Cut" | "NoCut") : "Cut";
      doc.nodes[id] = { id, kind: { Shape: a.kind }, transform: [1, 0, 0, 1, 0, 0], style, children: [], cut_line_type: cutLineType, material_preset: { state: "inherit" } };
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
      doc.nodes[id] = { id, kind: { Shape: { Path: { d: "" } } }, transform: [1, 0, 0, 1, 0, 0], style: DEFAULT_STYLE, children: [], cut_line_type: "Cut", material_preset: { state: "inherit" } };
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
    // Mirrors commands::set_cut_line_type: descends into containers, because the attribute is
    // read only on the shape that carries it — setting it on a Group alone would do nothing.
    set_cut_line_type: (a) => {
      const value = a.value as "Cut" | "NoCut";
      const ids = a.ids as number[];
      if (ids.length === 0) throw new Error("set_cut_line_type: EmptySelection");
      const seen = new Set<number>();
      const stack = [...ids];
      while (stack.length > 0) {
        const id = stack.pop()!;
        if (seen.has(id)) continue;
        seen.add(id);
        const n = doc.nodes[id];
        if (!n) throw new Error("set_cut_line_type: NotFound");
        if (typeof n.kind === "object" && n.kind !== null && "Shape" in (n.kind as object)) n.cut_line_type = value;
        else stack.push(...n.children);
      }
      return {};
    },
    // Mirrors commands::set_material_preset: writes the selection and nothing else, because a
    // material inherits and the planner resolves it. Descending here would be the bug the real
    // command was written to avoid.
    set_material_preset: (a) => {
      const value = a.value as PresetAssignment;
      const ids = a.ids as number[];
      if (ids.length === 0) throw new Error("set_material_preset: EmptySelection");
      for (const id of ids) {
        const n = doc.nodes[id];
        if (!n) throw new Error("set_material_preset: NotFound");
        n.material_preset = value;
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
      doc.nodes[id] = { id, kind: { Shape: { Path: { d: "" } } }, transform: [1, 0, 0, 1, 0, 0], style: DEFAULT_STYLE, children: [], cut_line_type: "Cut", material_preset: { state: "inherit" } };
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
  let planPasses: { key: string; enabled: boolean }[] = [];
  type CutRequest = {
    device_instance_id: string;
    doc_revision: string;
    grouping: Grouping;
    passes: {
      key: string;
      enabled: boolean;
      // Optional on the wire, not merely nullable: `ConfiguredPassDto::preset_id` is an
      // `Option<String>`, and serde reads a field the caller left out as `None`.
      preset_id?: string | null;
      speed: number | null;
      force: number | null;
      repeat_count: number | null;
    }[];
  };
  let lastCutRequest: CutRequest | null = null;
  let failNextResume = false;
  let failNextCut = false;
  let failNextPlan = false;
  // Presets as `cutplan::presets` keeps them: builtins ship per machine, the operator's own
  // entries live in one file beside them, and an entry replaces a builtin only when the whole pair
  // `(machine_id, id)` matches — an id is the operator's own string, so `my-vinyl` names one
  // material on a Cameo and another on a Puma (#153).
  type MaterialPreset = {
    id: string; name: string; machine_id: string;
    settings: { speed: number | null; force: number | null; repeat_count: number };
    builtin: boolean;
  };
  const BUILTIN_PRESETS: MaterialPreset[] = [
    { id: "cameo5-htv", name: "HTV", machine_id: "cameo5",
      settings: { speed: 5, force: 20, repeat_count: 1 }, builtin: true },
    // The Puma takes speed and force from its own panel, so its builtins name a material and
    // nothing else — the state the editor's preview has to read back as the panel's.
    { id: "puma-htv", name: "HTV", machine_id: "puma",
      settings: { speed: null, force: null, repeat_count: 1 }, builtin: true },
  ];
  let userPresets: MaterialPreset[] = opts?.seedUserPreset
    ? [{ id: "card-stock", name: "Card Stock", machine_id: "cameo5",
        settings: { speed: 6, force: 18, repeat_count: 1 }, builtin: false }]
    : [];
  const effectivePresets = (machineId: string): MaterialPreset[] => {
    const mine = userPresets.filter((p) => p.machine_id === machineId);
    const shipped = BUILTIN_PRESETS.filter(
      (b) => b.machine_id === machineId && !mine.some((p) => p.id === b.id),
    );
    return [...shipped, ...mine];
  };
  let failNextPresetSave = false;
  let failNextPresetList = false;
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
  // Presets hold on their own switch. The race they exist for is a list read for one cutter landing
  // after the operator aimed at another, and arming the plan hold with it would leave the dialog
  // without rows for the whole test.
  let holdingPresets = false;
  const heldPresets: (() => void)[] = [];
  Object.assign(window, {
    __armHold: () => { holding = true; },
    __releasePlans: () => release(heldPlans),
    __releaseTravel: () => release(heldTravel),
  });

  function ipcError(code: string, message: string) {
    return { code, message };
  }

  function planFromDoc(grouping: Grouping = "Color") {
    // Mirrors crates/cutplan/src/passes.rs's plan_passes_with: preorder walk, skip Shape leaf
    // nodes whose CutLineType is NoCut, and key the rest as the grouping asks — a colour
    // (stroke where visible, else fill; strict under Stroke and Fill, with 0-alpha counting as
    // absent), the resolved material, or `all` for one pass. Absence is its own token
    // (`no-color`, `no-preset`) because a preset id may be any string, so a preset called
    // `none` must not write what no preset at all writes.
    const byKey = new Map<string, { key: string; node_ids: number[] }>();
    let skipped = 0;
    const visible = (c: number | null | undefined) => (((c ?? 0) & 0xff) !== 0 ? c! : null);
    const colorKey = (n: Node) => {
      const stroke = visible(n.style.stroke);
      const fill = visible(n.style.fill);
      const c = grouping === "Stroke" ? stroke : grouping === "Fill" ? fill : stroke ?? fill;
      return c === null ? "no-color" : `color:${(c >>> 0).toString(16).padStart(8, "0")}`;
    };
    const walk = (id: number, inherited: string | null) => {
      const n = doc.nodes[id];
      if (!n) return;
      const a = n.material_preset;
      const material = a.state === "preset" ? a.id : a.state === "unassigned" ? null : inherited;
      const isShape = typeof n.kind === "object" && n.kind !== null && "Shape" in (n.kind as object);
      if (isShape) {
        if (n.cut_line_type === "NoCut") {
          skipped++;
        } else {
          const key =
            grouping === "Single" ? "all"
            : grouping === "Preset" ? (material === null ? "no-preset" : `preset:${material}`)
            : colorKey(n);
          const existing = byKey.get(key);
          if (existing) existing.node_ids.push(id);
          else byKey.set(key, { key, node_ids: [id] });
        }
      }
      for (const c of n.children) walk(c, material);
    };
    walk(doc.root, null);
    // starts is all-null on purpose: the fake carries no geometry to flatten, and null
    // is the real backend's no-outline case — so e2e renders exercise the preview's
    // bounds-corner badge fallback rather than a fixture pretending to be a blade path.
    const passes = [...byKey.values()].map((p) => ({
      key: p.key,
      shape_count: p.node_ids.length,
      node_ids: p.node_ids,
      starts: p.node_ids.map(() => null),
    }));
    // The snapshot itself is the revision, mirroring cutplan::doc_revision hashing
    // snapshot_json: a doc edited back to a previous state is not stale. A counter
    // bumped per command diverges on that, and silently goes stale-blind for any
    // command that mutates `doc` and forgets to bump (commit_transform did).
    // Travel by the same rule `travel_for_order` uses below - one segment per adjacent pair of
    // passes to be cut - because the real `plan_cut` returns the travel for the order it just
    // planned, and a fake that plans passes but never any travel between them cannot show a
    // preview going empty. Every pass a fresh plan produces is enabled.
    const travel = passes.slice(1).map((_, i) => [i, 0, i + 1, 0] as [number, number, number, number]);
    return { passes, skipped_not_cut: skipped, doc_revision: JSON.stringify(doc), travel };
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
      // Mirrors `DeviceManager`'s worker: a Connect is refused unless the manager is disconnected
      // or failed (`crates/driver-core/src/manager.rs`), so aiming at a second *local* cutter means
      // letting go of the first — while a failed one, which holds nothing, can be aimed away from
      // directly. A fake that switched outright let tests exercise a sequence production refuses
      // (Codex on PR #264). A cutter on a Cut Host is not that sequence:
      // `DeviceManagerHandle::connect` releases the local manager and records the aim, with no
      // transport of its own to open.
      const holdsATransport = status.phase !== "Disconnected" && status.phase !== "Failed";
      if (connected && connected.host === null && info.host === null
          && connected.instance_id !== info.instance_id && holdsATransport) {
        throw ipcError("device_error", "Busy");
      }
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
    plan_cut: (a) => {
      // A planner that refuses is an ordinary outcome — a font that will not resolve, a shape
      // with no outline — and the dialog has to survive one without lying about what it will
      // cut next.
      if (failNextPlan) {
        failNextPlan = false;
        throw ipcError("plan_error", "shape #2: no fonts are installed on this system");
      }
      // Answered from the document as it is *now*, like the real command, then parked if
      // the test has armed the hold: which of a replan and an older reorder settles first
      // is the whole subject of the race test, and a timing race cannot state it.
      const plan = planFromDoc(a.grouping as Grouping);
      if (!holding) return plan;
      return new Promise((resolve) => heldPlans.push(() => resolve(plan)));
    },
    // Mirrors device::travel_for_order's contract, not its geometry: the same stale-plan
    // refusal, the same exact-once identity check over the requested keys, then synthetic
    // segments (one per adjacent pair of *enabled* passes, x encoding the position in the
    // order) — the real command does not route the head to a pass that will not be cut.
    // Received lists are recorded on `window.__travelRequests` so a test can assert what the
    // dialog asked for; travel itself lands on a canvas Playwright cannot read.
    travel_for_order: (a) => {
      const passes = a.passes as { key: string; enabled: boolean }[];
      const grouping = a.grouping as Grouping;
      // The page's own hook object, which only this fake and the tests reading it touch. Named
      // rather than cast inline at each use: `window` genuinely has no type for a property the
      // test harness invents, and one reason beats two identical assertions.
      const hooks = window as unknown as { __travelRequests?: typeof passes[] };
      hooks.__travelRequests ??= [];
      hooks.__travelRequests.push(passes);
      const settle = () => {
        // Decided at settle time, like the real command — a request issued before a replan is
        // stale even if it settles after one.
        const plan = planFromDoc(grouping);
        if (plan.doc_revision !== a.docRevision) {
          throw ipcError("stale_plan", "document changed since the cut was planned; replan");
        }
        // The list must name each planned pass exactly once. Without this the fake accepts
        // rows from a previous grouping and the suite stays green on a frontend that cannot
        // work — which is the whole reason the dialog installs a plan atomically.
        const remaining = plan.passes.map((p) => p.key);
        for (const pass of passes) {
          const i = remaining.indexOf(pass.key);
          if (i === -1) {
            throw plan.passes.some((p) => p.key === pass.key)
              ? ipcError("plan_mismatch", "the requested pass list does not name every planned pass exactly once")
              : ipcError("unknown_pass", `no planned pass is called ${pass.key}`);
          }
          remaining.splice(i, 1);
        }
        if (remaining.length > 0) {
          throw ipcError("plan_mismatch", "the requested pass list does not name every planned pass exactly once");
        }
        const cut = passes.filter((p) => p.enabled);
        return cut.slice(1).map((_, i) => [i, 0, i + 1, 0] as [number, number, number, number]);
      };
      if (!holding) return settle();
      return new Promise((resolve, reject) => heldTravel.push(() => {
        try { resolve(settle()); } catch (e) { reject(e); }
      }));
    },
    cut: (a) => {
      const request = a.request as CutRequest;
      if (!connected) throw ipcError("not_connected", "no device connected");
      if (connected.instance_id !== request.device_instance_id) {
        throw ipcError("device_mismatch", "connected device changed since planning");
      }
      // Refused before the revision, machine and pass-key checks below, mirroring `prepare_cut`,
      // which resolves an enabled pass's preset before it parses the revision or plans. Named is
      // spelled out rather than tested for truth, in both directions: an empty id names a preset
      // and must be refused, while an omitted field is the `None` serde reads and must not be —
      // a fake that diverges either way is a green test for a cut the backend does not make.
      const available = effectivePresets(connected.machine_id);
      for (const pass of request.passes) {
        const named = pass.preset_id;
        if (pass.enabled && named !== null && named !== undefined
          && !available.some((p) => p.id === named)) {
          throw ipcError("unknown_preset",
            `this cut uses the material preset \`${named}\`, which is not available for this machine; pick another for that pass`);
        }
      }
      const plan = planFromDoc(request.grouping);
      if (plan.doc_revision !== request.doc_revision) {
        throw ipcError("stale_plan", "document changed since the cut was planned; replan");
      }
      if (doc.machine && doc.machine.id !== connected.machine_id) {
        throw ipcError("machine_mismatch", "document is set up for a different machine");
      }
      // A key this plan does not have is refused here too, so rows from a previous grouping
      // cannot cut the wrong shapes just because the fake was more forgiving than Rust.
      for (const pass of request.passes) {
        if (!plan.passes.some((p) => p.key === pass.key)) {
          throw ipcError("unknown_pass", `no planned pass is called ${pass.key}`);
        }
      }
      planPasses = request.passes;
      jobId = nextJobId++;
      const enabledIndices = planPasses.map((p, i) => (p.enabled ? i : -1)).filter((i) => i >= 0);
      if (enabledIndices.length === 0) throw ipcError("nothing_to_cut", "no enabled passes");
      // Recorded once nothing can still refuse the request, so the hook answers the cut that was
      // accepted rather than the last one attempted.
      lastCutRequest = request;
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
    __test_fail_next_plan: () => {
      failNextPlan = true;
      return {};
    },
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
    // Mirrors `desktop::device::list_presets`: this machine's builtins, minus any the operator's
    // own entries shadow by pair, plus those entries. The cut handler resolves against this same
    // list so a preset the dialog just saved can be cut.
    list_presets: (a) => {
      // Test hook: a read that fails *after* a write that did not is the interleaving the write and
      // the refresh are held apart for.
      if (failNextPresetList) {
        failNextPresetList = false;
        throw ipcError("preset_error", "Corrupt(\"the presets file could not be read\")");
      }
      const list = effectivePresets(a.machineId as string);
      if (!holdingPresets) return list;
      // Executor form, like the parked plan and travel replies above: the UI's `lib` is older than
      // `Promise.withResolvers`, and widening it for a fake is the wrong end of the trade.
      return new Promise<MaterialPreset[]>((resolve) => heldPresets.push(() => resolve(list)));
    },
    // The bounds `cutplan::preflight::SETTINGS_RANGES` publishes, restated here because a fake has
    // to answer something; the casing and the numbers are pinned on the Rust side.
    settings_ranges: () => ({
      speed: { min: 1, max: 30 },
      force: { min: 1, max: 33 },
      repeatCount: { min: 1, max: 10 },
    }),
    // Every refusal `desktop::device::save_preset` makes, because the editor is what must never
    // send one: an entry under a builtin's pair shadows a shipped material with no way back, an
    // id-less entry is dropped on load (a save the operator never gets back), and a setting out of
    // range is refused at cut time instead. A fake more forgiving than Rust would let the editor
    // ship any of those green.
    save_preset: (a) => {
      const p = a.p as MaterialPreset;
      if (failNextPresetSave) {
        failNextPresetSave = false;
        throw ipcError("preset_error", "Io(\"the presets file could not be written\")");
      }
      // In the backend's order: what the entry *is* before what it holds, so a test cannot pass
      // against a precedence production does not use (CodeRabbit on PR #264).
      if (p.id === "" || p.machine_id === "") {
        throw ipcError("invalid_preset", "a material preset needs an id and the machine it is for");
      }
      if (BUILTIN_PRESETS.some((b) => b.machine_id === p.machine_id && b.id === p.id)) {
        throw ipcError("builtin_preset",
          `\`${p.id}\` is a material preset that ships with the app; save your own under a different id`);
      }
      if (p.name.trim() === "") {
        throw ipcError("invalid_preset", "a material preset needs a name");
      }
      const range = { speed: [1, 30], force: [1, 33], repeat_count: [1, 10] } as const;
      for (const field of ["speed", "force", "repeat_count"] as const) {
        const v = p.settings[field];
        if (v !== null && (v < range[field][0] || v > range[field][1])) {
          throw ipcError("invalid_preset", `${field} must be ${range[field][0]}..=${range[field][1]}`);
        }
      }
      userPresets = [
        ...userPresets.filter((u) => !(u.machine_id === p.machine_id && u.id === p.id)),
        { ...p, builtin: false },
      ];
      return null;
    },
    // Named rather than silent when it removed nothing, as Rust is: a delete that reports success
    // having deleted nothing is how the editor would come to show a preset that is still there.
    delete_preset: (a) => {
      const machineId = a.machineId as string;
      const id = a.id as string;
      const before = userPresets.length;
      userPresets = userPresets.filter((u) => !(u.machine_id === machineId && u.id === id));
      if (userPresets.length === before) {
        if (BUILTIN_PRESETS.some((b) => b.machine_id === machineId && b.id === id)) {
          throw ipcError("builtin_preset", `\`${id}\` ships with the app, so there is nothing of yours to delete`);
        }
        throw ipcError("unknown_preset", `no material preset \`${id}\` is saved for \`${machineId}\``);
      }
      return null;
    },
    // Test hook (no production counterpart): arms a one-shot failure for the next preset write, so
    // a test can prove a refused save keeps the operator's edit on screen.
    __test_fail_next_preset_save: () => {
      failNextPresetSave = true;
      return null;
    },
    __test_fail_next_preset_list: () => {
      failNextPresetList = true;
      return null;
    },
    // Test hook (no production counterpart): the request the last accepted cut arrived with, so a
    // test can state which preset id the dialog sent rather than which one it displayed.
    __test_last_cut_request: () => lastCutRequest,
    // Test hooks (no production counterpart): park every `list_presets` reply from here, then let
    // them all land — the only way to state that a list read for one cutter arrives after the
    // operator has aimed at another.
    __test_hold_presets: () => {
      holdingPresets = true;
      return null;
    },
    // Releasing ends the window as well as draining it: a test that carries on aiming at cutters
    // afterwards wants their lists answered, not parked behind a switch nobody turned off.
    __test_release_presets: () => {
      holdingPresets = false;
      return release(heldPresets);
    },
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

// The operator-facing half of #148: the picker, the replan it triggers, and a row named for
// what it holds rather than for a colour it does not have. Nothing else in this suite selects a
// grouping, so a picker wired to a mode the backend ignores would leave every other test green.
test("changing the grouping replans and renames the passes", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByLabel("Group passes by").selectOption("Single");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
  await expect(page.getByText("Every cut shape")).toBeVisible();

  // And back again: switching modes replans each time rather than keeping the first answer.
  await page.getByLabel("Group passes by").selectOption("Stroke");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
});

// The race the dialog's installed-plan state exists to prevent: while a replan is parked, the
// rows on screen still belong to the previous grouping, and sending them under the new one
// would cut whatever that mode happens to key the same way. Cut has to be unavailable until
// the new plan lands — a fact only a held reply can state.
test("a cut cannot be sent with rows from the previous grouping", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();

  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByLabel("Group passes by").selectOption("Single");
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeDisabled();
  // Still showing the old mode's rows, which is exactly why Cut is unavailable.
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.evaluate(() => (window as unknown as { __releasePlans: () => void }).__releasePlans());
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
});

// Greptile's P1 on this PR, with its own Playwright repro: a replan that fails leaves the previous
// plan installed and cuttable, and the picker goes back to its mode - but travel was cleared on the
// way out and nothing brought it back. The operator was then offered a cut whose preview showed no
// travel at all, while the cut itself would travel exactly as before. The preview's accessible name
// is what makes the two states tellable apart from outside.
test("a rejected replan keeps the travel it was showing", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  // Two passes means one move between them - the travel this test is about.
  const preview = page.getByRole("img", { name: /Cut preview/ });
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");

  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByLabel("Group passes by").selectOption("Single");

  // The plan failed, so the previous one is still in force: same rows, same mode, still cuttable.
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByLabel("Group passes by")).toHaveValue("Color");
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
  // ...and the preview still describes the arrangement that cut would use.
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");
});

// Codex's finding on the first fix for the test above: restoring travel from a value captured when
// the replan started only works for one replan. A second replan beginning *while* the first is still
// out captures the already-cleared travel and restores that empty value on failure - so the preview
// went empty anyway, while the plan that was never replaced stayed installed and cuttable. Travel
// now lives in the installed plan, so a failure has nothing to restore and cannot lose it.
//
// The picker is disabled during a replan, so the two overlapping replans arrive the way an operator
// would actually produce them: through the stale-plan banner, whose Replan is deliberately still
// pressable - the banner is how a wedged plan gets unwedged, and a second press must not make
// things worse than the first.
test("a replan failing while another is parked keeps the installed plan's travel", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  const preview = page.getByRole("img", { name: /Cut preview/ });
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");

  // Stale the plan so the banner - and its Replan - are on screen.
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
      "commit_transform",
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
    ),
  );
  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Document changed since this plan was made.")).toBeVisible();

  // First Replan is parked in flight: the window in which travel used to be cleared.
  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByRole("button", { name: "Replan" }).click();

  // Second Replan, pressed inside that window, fails at once - `failNextPlan` is read before the
  // hold. Under the old design its rollback value was the cleared travel, not the installed one.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByRole("button", { name: "Replan" }).click();
  await expect(page.getByText(/no fonts are installed/)).toBeVisible();

  // Nothing was ever replaced, so the original plan is still the installed one - and the preview
  // must still describe it rather than showing a cut with no travel at all.
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");

  // The parked reply lands last and is superseded, so it changes nothing either.
  await page.evaluate(() => (window as unknown as { __releasePlans: () => void }).__releasePlans());
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");
});

// Codex's finding on the fix above, and the other half of it: moving travel into the plan removed
// the need to orphan pending travel when a replan *starts*, and leaving that bump in place turned it
// into the bug. A row edit's travel reply, orphaned on the way out of a replan that then fails, never
// lands - and the plan keeps the edited rows, so one enabled pass is left showing the travel of two,
// permanently. Orphaning belongs at installation, which is the one moment the rows a reply was
// computed for stop being the rows on screen.
test("a travel reply owed to a row edit still lands when a replan fails", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  const preview = page.getByRole("img", { name: /Cut preview/ });
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");

  // Disable a pass and park the travel reply it asks for: one pass left to cut means no travel
  // between passes, which is the answer this plan is owed.
  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByTestId("cut-pass-row").first().getByRole("checkbox").uncheck();
  await expect(preview).toHaveAccessibleName("Cut preview: 1 pass, 1 travel move");

  // Now a replan fails. The edited rows stay installed - so the parked reply is still the right
  // answer for them, and discarding it would leave the count above standing for good.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByLabel("Group passes by").selectOption("Single");
  await expect(page.getByText(/no fonts are installed/)).toBeVisible();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.evaluate(() => (window as unknown as { __releaseTravel: () => void }).__releaseTravel());
  await expect(preview).toHaveAccessibleName("Cut preview: 1 pass, 0 travel moves");
});

// The whole stale-material path through the real UI: the properties panel assigns a listed
// user preset, the dialog deletes it, preset grouping keeps the document's id on the pass, and
// the cut is refused because that id no longer resolves. Without this, `unknown_preset` could
// disappear while every saved-preset cut still passed.
test("a pass whose assigned preset was deleted is refused, not cut", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedUserPreset: true,
  });
  await page.goto("/");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);

  await page.getByTestId("layer-row").first().click();
  await expect(page.getByLabel("Material preset")).toBeVisible();
  await page.getByLabel("Material preset").selectOption("preset:card-stock");
  await expect(page.getByLabel("Material preset")).toHaveValue("preset:card-stock");

  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await page.getByLabel("Group passes by").selectOption("Preset");
  // One pass for the assigned material, one for everything that resolves to none.
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  // The pass resolves while the preset exists, and keeps the document's id once it is gone.
  await expect(page.getByTestId("cut-pass-row").first()).toContainText("Card Stock");
  await page.getByLabel("Preset to manage").selectOption("card-stock");
  await page.getByLabel("Delete preset").click();
  await expect(page.getByTestId("cut-pass-row").first()).toContainText("card-stock (unknown preset)");

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText(/not available for this machine/)).toBeVisible();
});

// The same refusal for the one id that reads like no id at all. Nothing else pins the fake's
// explicit null check: were it truthiness, an empty id would be cut with defaults here while
// `prepare_cut` refused it — a green e2e for a cut the real backend rejects.
test("an empty preset id is still named and refused", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedEmptyPresetAssignment: true,
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await page.getByLabel("Group passes by").selectOption("Preset");

  // Copilot on PR #272: the picker keys its options in the pass-key grammar, so the pass keyed on a
  // preset called `""` selects that preset rather than "No preset" — which a bare-id picker could
  // not express, since it had to spend the empty string on the absence.
  await expect(page.getByLabel("Preset for pass 1")).toHaveValue("preset:");
  await expect(page.getByTestId("cut-pass-row").first()).toContainText("(unknown preset)");
  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText(/material preset ``.*not available for this machine/)).toBeVisible();
});

/** A `cut` sent straight at the fake, carrying the plan's own revision unless the caller wants a
 *  stale one, with the refusal returned rather than thrown. These are requests the dialog cannot
 *  compose — it never sends an unplanned key, another machine's preset, or no `preset_id` field —
 *  so only a direct call can state what the backend would answer for one. `Single` plans one pass
 *  keyed `all`, which is what the callers below name. */
const cutDirect = (
  page: Page,
  request: { doc_revision?: string; grouping: string; passes: Record<string, unknown>[] },
) =>
  page.evaluate(async (req) => {
    // The fake's own channel, as everywhere else in this file: `__TAURI_INTERNALS__` is installed
    // by the fake, so the page's types do not know it.
    const internals = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
    };
    const plan = await internals.__TAURI_INTERNALS__.invoke("plan_cut", { grouping: req.grouping }) as {
      doc_revision: string;
    };
    try {
      return await internals.__TAURI_INTERNALS__.invoke("cut", {
        request: {
          device_instance_id: "usb:mock",
          doc_revision: req.doc_revision ?? plan.doc_revision,
          grouping: req.grouping,
          passes: req.passes,
        },
      });
    } catch (reason) {
      return reason;
    }
  }, request);

// Precedence: this request is stale *and* names a pass no plan has *and* names a missing preset.
// `no-preset` is a key the grammar accepts and a `Single` plan does not contain, so the payload is
// one the backend would really deserialize — a key like `missing` would die in `PassKey`'s parser
// before `prepare_cut` ran, and prove nothing about the order of its refusals. Were the preset
// check to drift below them, `stale_plan` would answer here while every UI test stayed green.
test("an unavailable preset is refused before later cut request checks", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();

  const error = await cutDirect(page, {
    doc_revision: "stale",
    grouping: "Single",
    passes: [{ key: "no-preset", enabled: true, preset_id: "gone", speed: null, force: null, repeat_count: null }],
  });
  expect(error).toMatchObject({ code: "unknown_preset" });
});

// A preset is machine-scoped, and `prepare_cut` filters the file to the connected cutter before it
// looks an id up — an operator's id is their own string, so the same one names different materials
// on two machines (#153). `puma-htv` is a material this Cameo cannot offer even though it is a
// listed builtin, which is what fails if the cut path ever resolves against every machine's
// entries: nothing else here would notice, since the tests above delete the entry outright.
test("a preset belonging to another cutter is not available to this one", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();

  const error = await cutDirect(page, {
    grouping: "Single",
    passes: [{ key: "all", enabled: true, preset_id: "puma-htv", speed: null, force: null, repeat_count: null }],
  });
  expect(error).toMatchObject({
    code: "unknown_preset",
    message: expect.stringContaining("`puma-htv`"),
  });
});

// The other half of the comparison the guard spells out: `ConfiguredPassDto::preset_id` is an
// `Option<String>`, so serde reads a pass carrying no such field as `None` and the cut proceeds. A
// fake that refused it would fail a request the backend accepts — the mirror wrong in the strict
// direction, which no UI test can reach because the dialog always sends the field.
test("a pass that carries no preset id at all is cut, not refused", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();

  const outcome = await cutDirect(page, {
    grouping: "Single",
    passes: [{ key: "all", enabled: true, speed: null, force: null, repeat_count: null }],
  });
  expect(outcome).toMatchObject({ job_id: expect.any(Number) });
});

// --- managing the operator's own material presets, in the dialog that cuts with them (#244) ---
//
// Each of these drives the real editor against the fake's preset store, which mirrors every
// refusal `desktop::device::save_preset` and `delete_preset` make. The invariants are the ones a
// unit test cannot reach: what is written, what comes back, and what is selected afterwards.

/** The fake's own hooks, reached through the channel the app itself invokes over. The cast is
 *  named here rather than repeated inside a callback: `__TAURI_INTERNALS__` is installed by the
 *  fake, so nothing in the page's own types knows about it. */
const callFake = (page: Page, cmd: string) =>
  page.evaluate((name) => {
    const internals = window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } };
    return internals.__TAURI_INTERNALS__.invoke(name);
  }, cmd);

/** Opens the Cut dialog on the local Cameo, which is the machine the presets below belong to. */
const openDialogOnCameo = async (page: Page) => {
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByLabel("Preset to manage")).toBeVisible();
};

test("a preset created in the cut dialog is offered to a pass and cut by its id", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Thick Card");
  await page.getByLabel("Preset speed").fill("7");
  await page.getByLabel("Preset force", { exact: true }).fill("21");
  await page.getByLabel("Preset repeat count").fill("2");
  await page.getByLabel("Save preset", { exact: true }).click();

  // Selected on what came back from the backend, not on what was typed: the file is what a preset
  // is, and the editor re-reads it after every write.
  await expect(page.getByLabel("Preset to manage")).toHaveValue("thick-card");
  await expect(page.getByTestId("preset-preview")).toHaveText("Cuts at speed 7, force 21, 2 passes.");
  // And a pass can now be cut with it, which is the whole point of managing them here.
  await expect(page.getByLabel("Preset for pass 1").locator("option")).toContainText(["No preset", "HTV", "Thick Card"]);
  await page.getByLabel("Preset for pass 1").selectOption("preset:thick-card");
  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();

  // Which pass carries it, not merely that some pass does: the dialog attaching the preset to the
  // wrong row is the regression this is here for, and the untouched row must still name none.
  const request = await callFake(page, "__test_last_cut_request") as {
    passes: { key: string; enabled: boolean; preset_id: string | null }[];
  };
  expect(request.passes.map((p) => [p.key, p.preset_id])).toEqual([
    ["color:ff0000ff", "thick-card"],
    ["color:00ff00ff", null],
  ]);
});

test("a built-in preset is read-only, and Save as Copy leaves it shipped", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("Preset to manage").selectOption("cameo5-htv");
  await expect(page.getByText("built-in — read-only")).toBeVisible();
  await expect(page.getByLabel("Preset name")).toBeDisabled();
  await expect(page.getByLabel("Delete preset")).toHaveCount(0);

  await page.getByLabel("Save as Copy").click();
  // A fresh id under a name of its own: an entry saved under `cameo5-htv` would shadow the shipped
  // material, and the backend refuses that pair outright.
  await expect(page.getByLabel("Preset to manage")).toHaveValue("htv-copy");
  await expect(page.getByLabel("Preset name")).toHaveValue("HTV (copy)");
  await expect(page.getByLabel("Preset name")).toBeEnabled();
  // The builtin is still listed, and still a builtin.
  await page.getByLabel("Preset to manage").selectOption("cameo5-htv");
  await expect(page.getByText("built-in — read-only")).toBeVisible();
  await expect(page.getByTestId("preset-preview")).toHaveText("Cuts at speed 5, force 20, one pass.");
});

test("renaming a preset keeps the id a pass and a document name it by", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Thick Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("thick-card");

  await page.getByLabel("Preset name").fill("Thin Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  // The name moved; the id did not. A PassKey is `preset:<id>` and a Node's assignment names the
  // same string, so an id that followed the name would orphan every document holding it.
  await expect(page.getByLabel("Preset to manage")).toHaveValue("thick-card");
  await expect(page.getByLabel("Preset to manage").locator("option")).toContainText(["HTV (built-in)", "Thin Card"]);
});

test("a name this cutter already uses is refused before anything is written", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("HTV");
  await expect(page.getByTestId("preset-error")).toContainText("built-in");
  await expect(page.getByLabel("Save preset", { exact: true })).toBeDisabled();

  // A name of its own clears it, and the same press then writes.
  await page.getByLabel("Preset name").fill("HTV, mine");
  await expect(page.getByTestId("preset-error")).toHaveCount(0);
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("htv-mine");
});

// CodeRabbit on the second push: the write and the re-read that follows it were caught together, so
// a read that failed after a write that did not reported a refused save, kept the draft as unsaved,
// and swallowed whatever was waiting on it.
test("a write that landed is not reported as refused when the re-read after it fails", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("card");

  // The save lands and the list read behind it does not: the section says the list could not be
  // read. What proves it was not read as a refused save is the draft — a refusal keeps it unsaved,
  // and the next guarded action would be parked to ask about it. This Close is not parked
  // (CodeRabbit: asserting on `preset-error` cannot fail here, since the editor is not rendered at
  // all while the list is unavailable).
  await page.getByLabel("Preset force", { exact: true }).fill("18");
  await callFake(page, "__test_fail_next_preset_list");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByText("Material presets are unavailable")).toContainText("could not be read");
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // And 18 is in the file: a fresh dialog reads the list again — no Connect, the cutter is still
  // connected — and the failure the section was showing is answered by that read.
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("force 18");

  // The same interleaving under the unsaved-changes decision: Save and continue writes, the read
  // behind it fails, the close it was blocking still happens — and 19 reaches the file too.
  await page.getByLabel("Preset force", { exact: true }).fill("19");
  await callFake(page, "__test_fail_next_preset_list");
  await page.getByRole("button", { name: "Close" }).click();
  await page.getByLabel("Save preset and continue").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("force 19");
});

test("deleting a preset selects a neighbour instead of showing settings that are gone", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  for (const name of ["Card", "Vinyl"]) {
    await page.getByLabel("New preset").click();
    await page.getByLabel("Preset name").fill(name);
    await page.getByLabel("Save preset", { exact: true }).click();
    await expect(page.getByLabel("Preset name")).toHaveValue(name);
  }

  await page.getByLabel("Preset to manage").selectOption("card");
  await page.getByLabel("Delete preset").click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("vinyl");
  await expect(page.getByLabel("Preset to manage").locator("option")).toContainText(["HTV (built-in)", "Vinyl"]);
  await expect(page.getByLabel("Preset to manage").locator("option")).toHaveCount(2);
});

// Codex on the third push: the write's own re-read reselects what it just saved, so a Save and
// continue whose continuation was "show me that other preset" landed on the saved one instead — the
// operator's next act undone by the write they asked for. Every existing decision test continued
// into a Close, where the dialog unmounts and the overwrite cannot be seen.
test("save and continue lands on the preset the operator asked for, not the one just written", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  for (const name of ["Card", "Vinyl"]) {
    await page.getByLabel("New preset").click();
    await page.getByLabel("Preset name").fill(name);
    await page.getByLabel("Save preset", { exact: true }).click();
    await expect(page.getByLabel("Preset name")).toHaveValue(name);
  }

  await page.getByLabel("Preset to manage").selectOption("card");
  await page.getByLabel("Preset speed").fill("14");
  await page.getByLabel("Preset to manage").selectOption("vinyl");
  await page.getByLabel("Save preset and continue").click();

  // Vinyl is what was asked for, and Card holds the 14 that was saved on the way there.
  await expect(page.getByLabel("Preset to manage")).toHaveValue("vinyl");
  await expect(page.getByLabel("Preset name")).toHaveValue("Vinyl");
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 14");
});

test("a write the backend refuses keeps the edit on screen and says why", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Thick Card");
  await page.getByLabel("Preset force", { exact: true }).fill("30");
  await callFake(page, "__test_fail_next_preset_save");
  await page.getByLabel("Save preset", { exact: true }).click();

  // The refusal is named, and the numbers are still there to try again with — they exist nowhere
  // else, so a cleared form would be the operator retyping them from memory.
  await expect(page.getByTestId("preset-error")).toContainText("presets file could not be written");
  await expect(page.getByLabel("Preset name")).toHaveValue("Thick Card");
  await expect(page.getByLabel("Preset force", { exact: true })).toHaveValue("30");

  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("thick-card");
  await expect(page.getByTestId("preset-error")).toHaveCount(0);
});

test("an unsaved preset edit has to be decided before the dialog closes", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Preset speed").fill("9");
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByText("Unsaved changes to the preset")).toBeVisible();

  // Keep editing leaves them exactly where they were, dialog and draft alike.
  await page.getByLabel("Keep editing the preset").click();
  await expect(page.getByLabel("Preset speed")).toHaveValue("9");

  // Save and continue does both: the entry is written, and then the close it was blocking happens.
  await page.getByRole("button", { name: "Close" }).click();
  await page.getByLabel("Save preset and continue").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // Reopening proves the save reached the backend rather than the dialog's own memory. No Connect
  // this time: the cutter is still connected, and the dialog seeds itself from the manager's cache
  // — pressing the first Connect on screen would aim at the *other* local cutter.
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 9");
});

test("discarding an unsaved edit writes nothing and lets the interrupted action through", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("card");

  await page.getByLabel("Preset speed").fill("12");
  // Selecting another preset is a discard just as closing is, so it is asked about the same way.
  await page.getByLabel("Preset to manage").selectOption("cameo5-htv");
  await expect(page.getByText("Unsaved changes to the preset")).toBeVisible();
  await page.getByLabel("Discard preset changes and continue").click();

  await expect(page.getByText("built-in — read-only")).toBeVisible();
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed from the cutter's panel");
});

test("a preset belongs to one cutter: aiming at another shows that machine's entries", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("My Vinyl");
  await page.getByLabel("Preset speed").fill("7");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("my-vinyl");

  // Aiming at a second local cutter means letting go of the first: `DeviceManager` refuses a
  // Connect while it holds a transport, so the dialog's Disconnect is the way across. The Puma's
  // list is its own — an id is the operator's own string, so one machine's entry must not appear,
  // or be editable, under another (#153).
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("button", { name: "Connect", exact: true }).nth(1).click();
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "HTV (built-in)" })).toHaveCount(1);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "My Vinyl" })).toHaveCount(0);

  // And the Cameo's entry is untouched by the trip, settings and all.
  await page.getByLabel("Disconnect serial:/dev/mock0").click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await page.getByLabel("Preset to manage").selectOption("my-vinyl");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 7");
});

// Greptile's P1 on the first push: a write leaves a `list_presets` out, and its reply installed the
// list *and* re-derived the draft from it with nothing asking which cutter it was read for. Aim at
// another cutter in that window and the editor showed the previous machine's entry — and a save
// would then have written that draft under the new machine's id.
//
// Reached through a delete rather than a save: a save leaves the draft dirty until its reply lands,
// so the unsaved-changes guard holds the cutter change back. A delete leaves nothing unsaved, so
// the aim really can move while the list is still out.
test("a preset list still owed to the previous cutter is not shown against this one", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  for (const name of ["My Vinyl", "Card"]) {
    await page.getByLabel("New preset").click();
    await page.getByLabel("Preset name").fill(name);
    await page.getByLabel("Save preset", { exact: true }).click();
    await expect(page.getByLabel("Preset name")).toHaveValue(name);
  }

  // Park every list reply, delete the Cameo's entry, then let go of the Cameo and aim at the Puma
  // while that list is still out — the sequence production allows, since a Connect is refused while
  // the manager holds a transport. The Puma's own read parks behind it, so both land on release.
  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Preset to manage").selectOption("my-vinyl");
  await page.getByLabel("Delete preset").click();
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("button", { name: "Connect", exact: true }).nth(1).click();
  await callFake(page, "__test_release_presets");

  // The Cameo's reply is inert: no draft of its material, and none of its entries in the picker.
  await expect(page.getByText("Choose a preset to edit")).toBeVisible();
  await expect(page.getByLabel("Preset name")).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "Card" })).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "HTV (built-in)" })).toHaveCount(1);

  // And the delete that was in flight did reach the Cameo's file, where it belonged.
  await page.getByLabel("Disconnect serial:/dev/mock0").click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "My Vinyl" })).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "Card" })).toHaveCount(1);
});

// Round 6 on PR #264: three findings, one cause. Every guard above was keyed on the machine id, and
// a machine id is not an aim — two aims at the same cutter share it. So a list read for a previous
// connection installed as though it were this one's (Copilot), and a continuation captured under
// that aim restored its entry as this aim's draft, which the next save wrote under this machine's id
// (Greptile, P1). The section is keyed on an aim generation now, the way the plan and the travel
// already are.
test("a preset list owed to a previous connection to the same cutter is not taken for this one", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("card");

  // Park every list reply, then save — that write's own re-read is now owed to *this* connection —
  // and let go of the Cameo and aim at it again. Both replies name the same machine, so nothing but
  // the aim tells them apart, and the older one would re-derive the draft it was written for.
  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Preset speed").fill("16");
  await page.getByLabel("Save preset", { exact: true }).click();
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();

  // This aim has read nothing yet: the editor is withheld, whatever the previous connection left
  // behind. Rendered from that cached list, New would mint an id against entries nobody re-read.
  await expect(page.getByText("Reading this cutter's presets…")).toBeVisible();
  await expect(page.getByLabel("New preset")).toHaveCount(0);

  // Released together, oldest first. The previous connection's reply is inert: the editor opens on
  // this aim's list with nothing selected, rather than back on the entry that write had settled.
  await callFake(page, "__test_release_presets");
  await expect(page.getByLabel("New preset")).toBeEnabled();
  await expect(page.getByText("Choose a preset to edit")).toBeVisible();
  await expect(page.getByLabel("Preset name")).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "Card" })).toHaveCount(1);

  // And the save that was in flight did land, so what is inert is the reply, not the write.
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 16");
});

test("a save's continuation is dropped when the cutter changed before it could run", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  for (const name of ["Card", "Vinyl"]) {
    await page.getByLabel("New preset").click();
    await page.getByLabel("Preset name").fill(name);
    await page.getByLabel("Save preset", { exact: true }).click();
    await expect(page.getByLabel("Preset name")).toHaveValue(name);
  }

  // Dirty on Card, ask for Vinyl, and answer Save and continue with the list replies parked: the
  // write lands, the continuation is still owed, and the aim moves to the Puma before it can run.
  await page.getByLabel("Preset to manage").selectOption("card");
  await page.getByLabel("Preset speed").fill("13");
  await page.getByLabel("Preset to manage").selectOption("vinyl");
  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Save preset and continue").click();
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("button", { name: "Connect", exact: true }).nth(1).click();
  await callFake(page, "__test_release_presets");

  // Nothing of the Cameo's arrives on the Puma: no draft, and no Cameo entry in its picker. The
  // continuation named Vinyl by the Cameo's list, and run here it would have become the Puma's
  // draft — then the Puma's entry under the next save.
  await expect(page.getByText("Choose a preset to edit")).toBeVisible();
  await expect(page.getByLabel("Preset name")).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "Vinyl" })).toHaveCount(0);

  // And the save that carried the continuation did land on the Cameo, where it was aimed.
  await page.getByLabel("Disconnect serial:/dev/mock0").click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 13");
});

// Codex's findings on the second push, both about an action that replaces the draft without the
// unsaved-changes decision: Duplicate writes from the *stored* entry (so it would drop the edit, or
// copy a version that no longer exists), and Delete replaces the draft with a neighbour's. Neither
// is offered while there is an edit to lose. The editor itself is withheld until the aimed cutter's
// own list has arrived, because that list is what a new entry's name and id have to avoid.
test("an unsaved edit withholds the actions that would discard it, and an unread list withholds the editor", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Duplicate preset")).toBeEnabled();
  await expect(page.getByLabel("Delete preset")).toBeEnabled();

  await page.getByLabel("Preset force", { exact: true }).fill("18");
  await expect(page.getByLabel("Duplicate preset")).toBeDisabled();
  await expect(page.getByLabel("Delete preset")).toBeDisabled();

  // Discarding is one of the two ways back, and both come back at once.
  await page.getByLabel("Discard preset changes").click();
  await expect(page.getByLabel("Duplicate preset")).toBeEnabled();
  await expect(page.getByLabel("Delete preset")).toBeEnabled();

  // With every list reply parked, aiming at the Puma leaves nothing to create against: no picker,
  // no New, and a line saying why.
  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("button", { name: "Connect", exact: true }).nth(1).click();
  await expect(page.getByText("Reading this cutter's presets…")).toBeVisible();
  await expect(page.getByLabel("New preset")).toHaveCount(0);

  await callFake(page, "__test_release_presets");
  await expect(page.getByLabel("New preset")).toBeEnabled();
});

// The pass rows' half of the same window, and the one #267 was filed for: the editor above is
// withheld until this aim's list arrives, but the rows are rendered from it regardless, and they
// name a material by looking its id up in exactly that list. Read as an ordinary empty list, a row
// tells the operator their pass has no material while `prepare_cut` would resolve it from the
// presets file and cut it.
test("a pass whose preset list has not arrived is named as unread, not as having no material", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedUserPreset: true,
  });
  await page.goto("/");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);
  await page.getByTestId("layer-row").first().click();
  await page.getByLabel("Material preset").selectOption("preset:card-stock");

  // Parked before the connect that asks for it, so every row below is rendered against a list
  // nobody has answered for — the seconds after a connect, held open.
  await page.getByRole("button", { name: "Cut" }).click();
  await callFake(page, "__test_hold_presets");
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await expect(page.getByText("Reading this cutter's presets…")).toBeVisible();
  await page.getByLabel("Group passes by").selectOption("Preset");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  const row = page.getByTestId("cut-pass-row").first();
  await expect(row).toContainText("card-stock (reading…)");
  // The picker carries the pass's own preset rather than nothing: a `select` whose value matches no
  // option renders blank, and blank is exactly what "No preset" looks like.
  await expect(page.getByLabel("Preset for pass 1")).toHaveValue("preset:card-stock");
  // And the repeat says nothing rather than one pass, which is a claim about the blade.
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  // Cutting stays available throughout: the backend resolves the material from the presets file,
  // and refuses by name when it cannot, so nothing here is a gate.
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();

  // The name and its settings arrive together, under the aim that asked for them.
  await callFake(page, "__test_release_presets");
  await expect(row).toContainText("Card Stock");
  await expect(page.getByLabel("Preset for pass 1")).toHaveValue("preset:card-stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("1");
});

test("the whole editor is operable from the keyboard alone", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  // Focus, then keys only: nothing below is a div with a click handler, which is the failure this
  // catches — an operator with the keyboard and no mouse can still reach every control.
  await page.getByLabel("New preset").focus();
  await page.keyboard.press("Enter");
  await page.getByLabel("Preset name").focus();
  await page.keyboard.type("Keyed Card");
  // Tab reaches the settings in the order they are read.
  await page.keyboard.press("Tab");
  await page.keyboard.type("8");
  await page.keyboard.press("Tab");
  await page.keyboard.type("22");
  await page.getByLabel("Save preset", { exact: true }).focus();
  await page.keyboard.press("Enter");

  await expect(page.getByLabel("Preset to manage")).toHaveValue("keyed-card");
  await expect(page.getByTestId("preset-preview")).toHaveText("Cuts at speed 8, force 22, one pass.");
});

// Greptile's P1 on the fifth push: a replan that *fails* leaves the previous plan in force —
// rows, revision and mode — but the picker had already moved to the mode nobody managed to plan.
// Cut then sent the old grouping while the operator read the new one off the screen. Nothing
// miscuts, which is what makes it worth a test: the lie is only visible on the dialog.
test("a grouping whose plan fails does not stay on the picker", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByLabel("Group passes by")).toHaveValue("Color");

  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByLabel("Group passes by").selectOption("Single");

  // The refusal is reported, the previous plan is still what would be cut, and the picker says
  // so rather than advertising the mode that failed.
  await expect(page.getByText(/no fonts are installed/)).toBeVisible();
  await expect(page.getByLabel("Group passes by")).toHaveValue("Color");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
});

// Greptile's P1 on PR #152, reproduced with a held reply: while a replacement plan is in flight
// the rows on screen still belong to the previous grouping, so an edit accepted there is
// discarded when the new plan installs — silently, with the operator's speed still on screen
// until it vanishes. Every row control is unavailable in that window.
test("row controls are unavailable while a replacement plan is in flight", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByLabel("Speed for pass 1")).toBeEnabled();

  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByLabel("Group passes by").selectOption("Single");

  // Still the old mode's two rows, and not one of their controls will take an edit.
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByLabel("Speed for pass 1")).toBeDisabled();
  await expect(page.getByLabel("Force for pass 1")).toBeDisabled();
  await expect(page.getByLabel("Repeat count for pass 1")).toBeDisabled();
  await expect(page.getByLabel("Preset for pass 1")).toBeDisabled();
  await expect(page.getByRole("checkbox", { name: "Enabled" }).first()).toBeDisabled();
  await expect(page.getByRole("button", { name: "Down" }).first()).toBeDisabled();

  await page.evaluate(() => (window as unknown as { __releasePlans: () => void }).__releasePlans());
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
  await expect(page.getByLabel("Speed for pass 1")).toBeEnabled();
});

// The whole operator-facing round trip: the properties panel's control, the real
// set_cut_line_type command, and the plan that then leaves the shape out. Nothing else in this
// suite reads the dialog's not-cut line, so a readout wired to a renamed field would render an
// `undefined` and every other test would still pass — hence the assertion on its full text,
// count and reason both. It is also the only exercise of planFromDoc's NoCut branch.
test("marking a shape No Cut drops its pass, and the dialog says why", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);

  await page.getByTestId("layer-row").first().click();
  const cuttable = page.getByRole("checkbox", { name: "Cut this shape" });
  await expect(cuttable).toBeChecked();
  await cuttable.uncheck();

  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
  await expect(page.getByText("Not cut: 1 shape marked No Cut")).toBeVisible();

  // Discriminating step: a command that ignored `value` and only ever wrote NoCut would pass
  // everything above. Marking it back has to restore the pass and empty the readout.
  await page.getByRole("button", { name: "Close" }).click();
  await cuttable.check();
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByText("Not cut: 0 shapes marked No Cut")).toBeVisible();

  // A selection whose shapes disagree is the one state the panel cannot answer with a plain
  // tick, and the direction it picks when clicked is not cosmetic: `checked={cutLineType !==
  // "NoCut"}` would render mixed as *checked*, so the click commits NoCut across the whole
  // selection and shapes silently stop cutting — with every other assertion here still green.
  await page.getByRole("button", { name: "Close" }).click();
  await cuttable.uncheck();
  await page.getByTestId("layer-row").nth(1).click({ modifiers: ["Shift"] });
  await expect(cuttable).toBeChecked({ indeterminate: true });

  await cuttable.click();
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByText("Not cut: 0 shapes marked No Cut")).toBeVisible();
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
  const requests = await page.evaluate(
    () => (window as unknown as { __travelRequests?: { key: string; enabled: boolean }[][] }).__travelRequests,
  );
  expect(requests).toEqual([[
    { key: "color:00ff00ff", enabled: true },
    { key: "color:ff0000ff", enabled: true },
  ]]);
});

// The head does not travel to a pass that will not be cut, so switching one off is a travel
// edit too — a preview that kept routing through it would draw motion the machine won't make.
test("disabling a pass replans travel without it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByTestId("cut-pass-row").first().getByRole("checkbox").uncheck();

  const requests = await page.evaluate(
    () => (window as unknown as { __travelRequests?: { key: string; enabled: boolean }[][] }).__travelRequests,
  );
  // Both passes still named — the disabled one is dropped from the travel by the planner,
  // not from the list, so a pass going missing stays distinguishable from a frontend bug.
  expect(requests).toEqual([[
    { key: "color:ff0000ff", enabled: false },
    { key: "color:00ff00ff", enabled: true },
  ]]);
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

  // The interleaving, without editing a row during a replan — which the dialog now refuses,
  // because rows belonging to a plan being replaced must not accept edits the arriving plan
  // discards. The reorder goes out *first* and is held, which is what the defect's own story
  // says anyway: "a reorder issued before Replan carries the old revision".
  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByRole("button", { name: "Down" }).first().click();
  await page.getByRole("button", { name: "Replan" }).click();

  await page.evaluate(() => (window as unknown as { __releasePlans: () => Promise<unknown> }).__releasePlans());
  await expect(banner).toHaveCount(0);

  // The late refusal for the revision the fresh plan replaced must not re-raise the banner.
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
  // Nothing has been refused yet, so the warning is nowhere — including over the two local
  // cutters, whose section has no host id at all for a refusal to match (#265).
  const forceWarning = page.getByText(/A cut may still be running on this Cut Host/);
  await expect(forceWarning).toHaveCount(0);

  await page.getByRole("button", { name: "Forget Workshop Pi" }).click();
  // The Rust side's own words, and the row still there (#94).
  await expect(page.getByText("this Cut Host could not be asked whether it is cutting")).toBeVisible();
  await expect(page.getByText("Workshop Pi")).toBeVisible();

  // Only now is the force on screen, and it says what is being accepted rather than asking
  // whether the operator is sure. Once, against the host that refused: the local section is still
  // listed below it and must not have grown a copy.
  await expect(forceWarning).toHaveCount(1);
  await page.getByRole("button", { name: "Discard Workshop Pi anyway" }).click();
  // A Pi that is gone for good must not become unforgettable — the row and its cutter both go.
  await expect(page.getByRole("button", { name: /^Forget/ })).toHaveCount(0);
  expect(await page.getByText("Workshop Pi").count()).toBe(0);
});

// The absence is its own test because every path to the warning goes through a host, and a desktop
// with none paired has no Forget to press: the defect it covers (#265) needed no interaction at all,
// so a test that starts by pairing something can never see it.
test("a desktop with no Cut Host paired shows no force-forget warning", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();

  // The local cutters are listed — the dialog is populated, so the warning's absence is about the
  // guard rather than about an empty device section.
  await expect(page.getByTestId("device-badge")).toHaveCount(2);
  await expect(page.getByText(/A cut may still be running on this Cut Host/)).toHaveCount(0);
  await expect(page.getByRole("button", { name: /anyway$/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /^Keep / })).toHaveCount(0);
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
