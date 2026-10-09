// SPDX-License-Identifier: GPL-3.0-or-later
import { test, expect, type Page } from "@playwright/test";
// Generated from the desktop's own generate_handler! registry by
// apps/desktop/tests/ipc_inventory.rs. Nothing else here calls a real #[tauri::command], so this
// is what the fake below checks command and argument names against (#85).
// The attribute is not decoration: Playwright loads this file as ESM, where a JSON import without
// one is a TypeError before any test is collected.
import ipcInventory from "../../ipc-inventory.json" with { type: "json" };

// Minimal in-memory fake Tauri backend. Runs inside the page (via addInitScript, so it
// can't close over anything outside itself) and mirrors the JSON shape produced by
// crates/document's Document::snapshot_json() — see App.tsx's DocSnapshot/buildScene,
// which is what actually parses this on the JS side.
function installMockTauri(opts?: { seedTwoColorRects?: boolean; failImagePreview?: boolean; dropTraceControl?: string; seedBusyHost?: boolean; seedRemoteConnected?: boolean; slowList?: boolean; failList?: boolean; noFonts?: boolean; seedMachine?: boolean; seedUserPreset?: boolean; seedEmptyPresetAssignment?: boolean; seedGroup?: boolean; seedAlignExtras?: boolean; seedCollapsedGroup?: boolean; collapsedScale?: number }) {
  type Style = { stroke: number | null; fill: number | null };
  type PresetAssignment = { state: "inherit" } | { state: "unassigned" } | { state: "preset"; id: string };
  type Node = { id: number; kind: unknown; transform: number[]; style: Style; children: number[]; cut_line_type: "Cut" | "NoCut"; material_preset: PresetAssignment };
  type Grouping = "Single" | "Color" | "Stroke" | "Fill" | "Preset";
  type WeedOptions = { margin_mm: number; lines: "None" | "Horizontal" | "Vertical" | "Both"; spacing_mm: number; clearance_mm: number };
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
  // A Group 30 mm right of the origin holding one 10 × 10 mm rect: a selected container commits as
  // its whole subtree, which the canvas has to preview and box the same way (Copilot on #298).
  if (opts?.seedGroup) {
    const groupId = nextId++;
    const childId = nextId++;
    doc.nodes[groupId] = { id: groupId, kind: "Group", transform: [1, 0, 0, 1, 30, 0], style: { stroke: null, fill: null }, children: [childId], cut_line_type: "Cut", material_preset: { state: "inherit" } };
    doc.nodes[childId] = { id: childId, kind: { Shape: { Rect: { w: 10, h: 10 } } }, transform: [1, 0, 0, 1, 0, 0], style: { stroke: 0xff0000ff, fill: null }, children: [], cut_line_type: "Cut", material_preset: { state: "inherit" } };
    doc.nodes[doc.root].children.push(groupId);
  }

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

  // After the rects: a Group of two 10 mm rects at (50, 20) and (70, 40), so its bounds are
  // 50..80 × 20..50 and no single shape gives them, and an empty Group with nothing to line up.
  if (opts?.seedAlignExtras) {
    const groupId = nextId++;
    const leftId = nextId++;
    const rightId = nextId++;
    const emptyId = nextId++;
    const rect = (id: number, x: number, y: number): Node => ({ id, kind: { Shape: { Rect: { w: 10, h: 10 } } }, transform: [1, 0, 0, 1, x, y], style: { stroke: 0x0000ffff, fill: null }, children: [], cut_line_type: "Cut", material_preset: { state: "inherit" } });
    doc.nodes[groupId] = { id: groupId, kind: "Group", transform: [1, 0, 0, 1, 0, 0], style: { stroke: null, fill: null }, children: [leftId, rightId], cut_line_type: "Cut", material_preset: { state: "inherit" } };
    doc.nodes[leftId] = rect(leftId, 50, 20);
    doc.nodes[rightId] = rect(rightId, 70, 40);
    doc.nodes[emptyId] = { id: emptyId, kind: "Group", transform: [1, 0, 0, 1, 0, 0], style: { stroke: null, fill: null }, children: [], cut_line_type: "Cut", material_preset: { state: "inherit" } };
    doc.nodes[doc.root].children.push(groupId, emptyId);
  }

  // A Group scaled to nothing, holding one rect: a move beneath it cannot be put into its space,
  // which is the geometry refusal `transform_nodes` makes after earlier entries already succeeded.
  if (opts?.seedCollapsedGroup) {
    const groupId = nextId++;
    const childId = nextId++;
    const k = opts.collapsedScale ?? 0;
    doc.nodes[groupId] = { id: groupId, kind: "Group", transform: [k, 0, 0, k, 0, 0], style: { stroke: null, fill: null }, children: [childId], cut_line_type: "Cut", material_preset: { state: "inherit" } };
    doc.nodes[childId] = { id: childId, kind: { Shape: { Rect: { w: 10, h: 10 } } }, transform: [1, 0, 0, 1, 0, 0], style: { stroke: 0xff0000ff, fill: null }, children: [], cut_line_type: "Cut", material_preset: { state: "inherit" } };
    doc.nodes[doc.root].children.push(groupId);
  }

  const unimplemented = (cmd: string): never => {
    throw new Error(`${cmd}: mocked command the e2e fake does not perform; implement it here to test it`);
  };

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    new_doc: () => {
      doc = freshDoc();
      return JSON.stringify(doc);
    },
    snapshot: () => {
      // One-shot, armed by a test: a refresh that fails after a commit already landed is the case
      // whose repair is the opposite of a refusal (silent-failure-hunter on #298).
      if (failNextSnapshot) {
        failNextSnapshot = false;
        throw new Error("snapshot unavailable");
      }
      // Read when the command runs, as the backend does, so a held answer still shows the document
      // as it stood then, even if a load has replaced it before the answer arrives.
      const json = JSON.stringify(doc);
      if (holdingSnapshots) return new Promise((resolve) => heldSnapshots.push(() => resolve(json)));
      return json;
    },
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
      // Composed in full: handles send scale and rotation, and a fake that kept only the
      // translation passes a frontend whose preview and commit disagree. Recorded so a test can
      // read the matrix.
      if (failNextCommit) {
        failNextCommit = false;
        throw new Error("transform refused");
      }
      const m = a.m as number[];
      const hooks = window as unknown as { __commitTransforms?: { ids: number[]; m: number[]; batch?: number }[] };
      hooks.__commitTransforms ??= [];
      hooks.__commitTransforms.push({ ids: a.ids as number[], m });
      return answer(() => applyTransforms([{ ids: a.ids as number[], m }]));
    },
    commit_transforms: (a) => {
      // Counted while unanswered: the real backend serialises commands, so two batches on the wire
      // at once is a frontend that stopped holding its queue (CodeRabbit on #301).
      const hooks = window as unknown as { __maxInFlightCommits?: number };
      inFlightCommits += 1;
      hooks.__maxInFlightCommits = Math.max(hooks.__maxInFlightCommits ?? 0, inFlightCommits);
      const answered = ((): Promise<unknown> => {
      // Mirrors commands::transform_each: every move in one undo, all or nothing, each entry on
      // what the earlier ones left, and nothing moved twice. Each entry is recorded with its batch,
      // so a test can tell one click from several. A refusal waits on a hold like an answer does,
      // so a test can queue edits behind a commit that will be refused.
      if (failNextCommit) {
        failNextCommit = false;
        return answer(() => { throw new Error("transform refused"); });
      }
      const moves = a.moves as { ids: number[]; m: number[] }[];
      // Checked before anything is recorded (all or nothing), and answered in turn like every
      // other outcome: the real backend serialises commands, so a refusal never overtakes a held
      // commit ahead of it (CodeRabbit on #301).
      // An empty batch or entry is EmptySelection in transform_each, refused whole (Copilot on #301).
      if (moves.length === 0 || moves.some((mv) => mv.ids.length === 0)) {
        return answer(() => { throw new Error("the selection has nothing this command can act on"); });
      }
      if (moves.some((mv) => mv.ids.some((id) => !doc.nodes[id]))) {
        return answer(() => { throw new Error("the node or machine this command names is not there"); });
      }
      // So is a transform that cannot be reversed, against the document as it stands, as the missing
      // node is: a batch Rust refuses whole must not reach the log and take a batch number. It is
      // staged again when answered, since a held commit ahead of it can still change the document.
      try {
        stageTransforms(moves);
      } catch (e) {
        return answer(() => { throw e; });
      }
      const hooks = window as unknown as { __commitTransforms?: { ids: number[]; m: number[]; batch?: number }[]; __batches?: number };
      hooks.__commitTransforms ??= [];
      hooks.__batches = (hooks.__batches ?? 0) + 1;
      for (const mv of moves) hooks.__commitTransforms.push({ ids: mv.ids, m: mv.m, batch: hooks.__batches });
      return answer(() => applyTransforms(moves));
      })();
      return answered.finally(() => { inFlightCommits -= 1; });
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
      const apply = () => {
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
      };
      // Held, it stands for any edit still on its way when a load is asked for (an import reading
      // its file): applied on release, so a load that did not wait for it lands first.
      if (!holdingEdits) return apply();
      inFlightEdits++;
      return new Promise((resolve, reject) =>
        heldEdits.push(() => {
          inFlightEdits--;
          try { resolve(apply()); } catch (e) { reject(e); }
        }));
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
      (window as unknown as { __docOrder?: string[] }).__docOrder?.push("import_svg");
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
      // The real backend runs commands one at a time, but only a frontend that waits keeps a load
      // from landing between a commit and the document it named (CodeRabbit on #301).
      const loads = window as unknown as { __loadDuringCommit?: boolean; __loadDuringEdit?: boolean; __loads?: number };
      if (inFlightCommits > 0) loads.__loadDuringCommit = true;
      if (inFlightEdits > 0) loads.__loadDuringEdit = true;
      loads.__loads = (loads.__loads ?? 0) + 1;
      (window as unknown as { __docOrder?: string[] }).__docOrder?.push("load_project");
      const load = () => {
        if (saved) doc = JSON.parse(JSON.stringify(saved));
        return JSON.stringify(doc);
      };
      // A real load takes as long as the project is big; held, a test can act while it runs.
      if (holdingLoads) return new Promise((resolve) => heldLoads.push(() => resolve(load())));
      return load();
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
    weed?: WeedOptions | null;
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
  // Set by a test that needs a held plan to fail when released rather than install: a replan that
  // is out while the operator keeps typing, and then refused.
  let refuseHeldPlans = false;
  const heldPlans: (() => void)[] = [];
  // The weed controls start from the defaults `settings_ranges` carries, so a test needs the window
  // before they arrive, and the case where they never do.
  let holdingRanges = false;
  let rangesFail = false;
  const heldRanges: (() => void)[] = [];
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
  // Transforms hold on their own switch too: the race they exist for is a second gesture pressed
  // while the first one's commit is still on the wire.
  let holdingCommits = false;
  const heldCommits: (() => void)[] = [];
  let failNextCommit = false;
  let inFlightCommits = 0;
  let holdingLoads = false;
  const heldLoads: (() => void)[] = [];
  let failNextSnapshot = false;
  let holdingEdits = false;
  const heldEdits: (() => void)[] = [];
  let inFlightEdits = 0;
  let holdingSnapshots = false;
  const heldSnapshots: (() => void)[] = [];
  Object.assign(window, {
    __holdSnapshots: () => { holdingSnapshots = true; },
    __holdEdits: () => { holdingEdits = true; },
    __releaseEdits: () => { holdingEdits = false; return release(heldEdits); },
    // Oldest first, one at a time: the race is which document's answer renders last.
    __releaseSnapshot: () => release(heldSnapshots.splice(0, 1)),
    __releaseLatestSnapshot: () => release(heldSnapshots.splice(-1, 1)),
    __failNextCommit: () => { failNextCommit = true; },
    __failNextSnapshot: () => { failNextSnapshot = true; },
    __holdCommits: () => { holdingCommits = true; },
    __holdLoad: () => { holdingLoads = true; },
    __releaseLoad: () => { holdingLoads = false; return release(heldLoads); },
    __releaseCommits: () => { holdingCommits = false; return release(heldCommits); },
    __armHold: () => { holding = true; },
    __releasePlans: () => release(heldPlans),
    __refuseHeldPlans: () => { refuseHeldPlans = true; },
    __holdRanges: () => { holdingRanges = true; },
    __releaseRanges: () => release(heldRanges),
    __failRanges: () => { rangesFail = true; },
    __releaseTravel: () => release(heldTravel),
  });

  // Mirrors crates/document/src/commands.rs transform_nodes_in, which transform_nodes and
  // transform_each share: `m` is world-space, so each node's local transform becomes
  // local · parentWorld · m · parentWorld⁻¹, and a node whose ancestor any entry selects is skipped,
  // since the ancestor already carries it. Composing `m` straight onto a nested node's local
  // transform lands it where the real backend never would (Copilot on #298).
  function applyTransforms(moves: { ids: number[]; m: number[] }[]) {
    for (const [id, t] of stageTransforms(moves)) doc.nodes[id].transform = t;
    return {};
  }

  // Every entry's result, or a throw for the whole batch. Nothing is written here.
  function stageTransforms(moves: { ids: number[]; m: number[] }[]) {
    const cmp = (p: number[], q: number[]) => [
      q[0] * p[0] + q[2] * p[1], q[1] * p[0] + q[3] * p[1],
      q[0] * p[2] + q[2] * p[3], q[1] * p[2] + q[3] * p[3],
      q[0] * p[4] + q[2] * p[5] + q[4], q[1] * p[4] + q[3] * p[5] + q[5],
    ];
    const inv = (p: number[]) => {
      const det = p[0] * p[3] - p[1] * p[2];
      // As Rust's `pw.inverse()` refusing: the whole batch goes, not just this entry. Exactly zero,
      // as `Affine::inverse` checks: a tolerance refused tiny scales Rust accepts (Copilot on #301).
      if (det === 0) throw new Error("something in the selection sits under a transform that cannot be reversed");
      const [ia, ib, ic, id] = [p[3] / det, -p[1] / det, -p[2] / det, p[0] / det];
      return [ia, ib, ic, id, -(ia * p[4] + ic * p[5]), -(ib * p[4] + id * p[5])];
    };
    // Staged, then published only if every entry succeeds: Rust collects every entry's update and
    // commits none on any refusal, so a later entry's failure must leave the earlier ones unwritten
    // (Copilot on #301). Later entries read the staged transforms, as Rust's read the transforms
    // the earlier ones wrote.
    const staged = new Map<number, number[]>();
    const transformOf = (id: number) => staged.get(id) ?? doc.nodes[id].transform;
    const parentOf = (id: number) => Object.values(doc.nodes).find((n) => n.children.includes(id))?.id;
    const worldOf = (id: number | undefined): number[] =>
      id === undefined ? [1, 0, 0, 1, 0, 0] : cmp(transformOf(id), worldOf(parentOf(id)));
    const selectedIds = new Set(moves.flatMap((mv) => mv.ids));
    const hasSelectedAncestor = (id: number) => {
      for (let p = parentOf(id); p !== undefined; p = parentOf(p)) if (selectedIds.has(p)) return true;
      return false;
    };
    for (const { ids, m } of moves) {
      for (const id of new Set(ids)) {
        // Checked again when answered: a node deleted while the batch was held is NotFound in
        // Rust, which refuses the whole batch rather than skipping it (Copilot on #301).
        if (!doc.nodes[id]) throw new Error("the node or machine this command names is not there");
        if (hasSelectedAncestor(id)) continue;
        const pw = worldOf(parentOf(id));
        staged.set(id, cmp(cmp(cmp(transformOf(id), pw), m), inv(pw)));
      }
    }
    return staged;
  }

  // Answered on a later task, as a real IPC round trip is: resolving in the same microtask burst
  // let a whole chain of queued commits finish before React rendered between them, which hid the
  // window where an intermediate snapshot renders while the next commit is on the wire.
  function answer<T>(run: () => T): Promise<T> {
    return new Promise((resolve, reject) => {
      const settle = () => {
        try { resolve(run()); } catch (e) { reject(e); }
      };
      if (holdingCommits) heldCommits.push(settle);
      else setTimeout(settle, 0);
    });
  }

  function ipcError(code: string, message: string) {
    return { code, message };
  }

  // Mirrors cutplan::weed. WEED_RANGES and WeedOptions::validate, refusing with the same sentences.
  const WEED_RANGES = { margin_mm: { min: 0.5, max: 50 }, spacing_mm: { min: 5, max: 500 }, clearance_mm: { min: 0.2, max: 20 } };
  function validateWeed(w: WeedOptions) {
    const check = (name: string, v: number, r: { min: number; max: number }) => {
      if (!(v >= r.min && v <= r.max)) throw ipcError("plan_error", `the weed ${name} must be ${r.min}–${r.max} mm`);
    };
    check("margin", w.margin_mm, WEED_RANGES.margin_mm);
    if (w.lines === "None") return;
    check("line spacing", w.spacing_mm, WEED_RANGES.spacing_mm);
    check("line clearance", w.clearance_mm, WEED_RANGES.clearance_mm);
    if (w.clearance_mm >= w.margin_mm) {
      throw ipcError("plan_error", `the weed line clearance (${w.clearance_mm} mm) must be less than the margin (${w.margin_mm} mm)`);
    }
  }

  // Mirrors cutplan::weed::weed_geometry for what the fixtures hold: axis-aligned rects. For a
  // closed axis-aligned rect, the band clipping plus the winding test reduce exactly to blocking
  // the rect's box grown by the clearance, so this is the real output, not an approximation of it.
  type Box = { x: number; y: number; w: number; h: number };
  function weedFor(boxes: Box[], w: WeedOptions): [number, number][][] {
    if (boxes.length === 0) return [];
    const left = Math.min(...boxes.map((b) => b.x)) - w.margin_mm;
    const top = Math.min(...boxes.map((b) => b.y)) - w.margin_mm;
    const right = Math.max(...boxes.map((b) => b.x + b.w)) + w.margin_mm;
    const bottom = Math.max(...boxes.map((b) => b.y + b.h)) + w.margin_mm;
    const c = w.clearance_mm;
    const out: [number, number][][] = [];
    // `at` is the line's position on one axis; each box blocks [lo, hi] on the other if it comes
    // within the clearance of the line.
    const pieces = (at: number, from: number, to: number, spans: { near: [number, number]; block: [number, number] }[]) => {
      const blocked = spans.filter((s) => at >= s.near[0] - c && at <= s.near[1] + c)
        .map((s) => [s.block[0] - c, s.block[1] + c]).sort((p, q) => p[0] - q[0]);
      const free: [number, number][] = [];
      let pos = from;
      for (const [a, b] of blocked) {
        if (a > pos) free.push([pos, Math.min(a, to)]);
        pos = Math.max(pos, b);
        if (pos >= to) break;
      }
      if (pos < to) free.push([pos, to]);
      return free.filter(([a, b]) => b - a >= 2);
    };
    const steps = (from: number, to: number) => {
      const v: number[] = [];
      for (let k = 1; from + k * w.spacing_mm < to - 1e-9; k++) v.push(from + k * w.spacing_mm);
      return v;
    };
    if (w.lines === "Horizontal" || w.lines === "Both") {
      const spans = boxes.map((b) => ({ near: [b.y, b.y + b.h] as [number, number], block: [b.x, b.x + b.w] as [number, number] }));
      for (const y of steps(top, bottom)) for (const [a, b] of pieces(y, left, right, spans)) out.push([[a, y], [b, y]]);
    }
    if (w.lines === "Vertical" || w.lines === "Both") {
      const spans = boxes.map((b) => ({ near: [b.x, b.x + b.w] as [number, number], block: [b.y, b.y + b.h] as [number, number] }));
      for (const x of steps(left, right)) for (const [a, b] of pieces(x, top, bottom, spans)) out.push([[x, a], [x, b]]);
    }
    out.push([[left, top], [right, top], [right, bottom], [left, bottom], [left, top]]);
    return out;
  }

  function planFromDoc(grouping: Grouping = "Color", weed: WeedOptions | null = null) {
    if (weed) validateWeed(weed);
    // Mirrors crates/cutplan/src/passes.rs's plan_passes_with: preorder walk, skip Shape leaf
    // nodes whose CutLineType is NoCut, and key the rest as the grouping asks — a colour
    // (stroke where visible, else fill; strict under Stroke and Fill, with 0-alpha counting as
    // absent), the resolved material, or `all` for one pass. Absence is its own token
    // (`no-color`, `no-preset`) because a preset id may be any string, so a preset called
    // `none` must not write what no preset at all writes.
    const byKey = new Map<string, { key: string; node_ids: number[]; boxes: Box[] }>();
    let skipped = 0;
    const visible = (c: number | null | undefined) => (((c ?? 0) & 0xff) !== 0 ? c! : null);
    const colorKey = (n: Node) => {
      const stroke = visible(n.style.stroke);
      const fill = visible(n.style.fill);
      const c = grouping === "Stroke" ? stroke : grouping === "Fill" ? fill : stroke ?? fill;
      return c === null ? "no-color" : `color:${(c >>> 0).toString(16).padStart(8, "0")}`;
    };
    const mul = (p: number[], q: number[]) => [
      q[0] * p[0] + q[2] * p[1], q[1] * p[0] + q[3] * p[1],
      q[0] * p[2] + q[2] * p[3], q[1] * p[2] + q[3] * p[3],
      q[0] * p[4] + q[2] * p[5] + q[4], q[1] * p[4] + q[3] * p[5] + q[5],
    ];
    // A Rect's world box, for the weed. Other kinds carry no geometry here, so they add none.
    const worldBox = (n: Node, world: number[]): Box | null => {
      const r = (n.kind as { Shape?: { Rect?: { x?: number; y?: number; w: number; h: number } } }).Shape?.Rect;
      if (!r) return null;
      const [x0, y0] = [r.x ?? 0, r.y ?? 0];
      const pts = [[x0, y0], [x0 + r.w, y0], [x0, y0 + r.h], [x0 + r.w, y0 + r.h]]
        .map(([x, y]) => [world[0] * x + world[2] * y + world[4], world[1] * x + world[3] * y + world[5]]);
      const xs = pts.map((p) => p[0]);
      const ys = pts.map((p) => p[1]);
      return { x: Math.min(...xs), y: Math.min(...ys), w: Math.max(...xs) - Math.min(...xs), h: Math.max(...ys) - Math.min(...ys) };
    };
    const walk = (id: number, inherited: string | null, parentWorld: number[] = [1, 0, 0, 1, 0, 0]) => {
      const n = doc.nodes[id];
      if (!n) return;
      const world = mul(n.transform, parentWorld);
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
          const box = worldBox(n, world);
          const existing = byKey.get(key);
          if (existing) {
            existing.node_ids.push(id);
            if (box) existing.boxes.push(box);
          } else byKey.set(key, { key, node_ids: [id], boxes: box ? [box] : [] });
        }
      }
      for (const c of n.children) walk(c, material, world);
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
      weed: weed ? weedFor(p.boxes, weed) : [],
    }));
    // The snapshot itself is the revision, mirroring cutplan::doc_revision hashing
    // snapshot_json: a doc edited back to a previous state is not stale. A counter
    // bumped per command diverges on that, and silently goes stale-blind for any
    // command that mutates `doc` and forgets to bump (commit_transform did).
    // Travel by the same rule `travel_for_order` uses below - one segment per adjacent pair of
    // passes to be cut - because the real `plan_cut` returns the travel for the order it just
    // planned, and a fake that plans passes but never any travel between them cannot show a
    // preview going empty. Every pass a fresh plan produces is enabled.
    const travel = syntheticTravel(passes);
    return { passes, skipped_not_cut: skipped, doc_revision: JSON.stringify(doc), travel };
  }

  // One synthetic segment per adjacent pair of passes cut, plus one per weed polyline, which the
  // real `travel_moves` visits after each pass's shapes. The fake has no blade path to put them on.
  function syntheticTravel(cut: { weed: [number, number][][] }[]) {
    const between = cut.slice(1).map((_, i) => [i, 0, i + 1, 0] as [number, number, number, number]);
    const toWeed = cut.flatMap((p) => p.weed.map(() => [0, 0, 0, 0] as [number, number, number, number]));
    return [...between, ...toWeed];
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
        throw ipcError("device_busy", "the cutter cannot do that right now");
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
      const plan = planFromDoc(a.grouping as Grouping, (a.weed as WeedOptions | null | undefined) ?? null);
      if (!holding) return plan;
      return new Promise((resolve, reject) => heldPlans.push(() =>
        refuseHeldPlans ? reject(ipcError("plan_error", "shape #2: no fonts are installed on this system")) : resolve(plan)));
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
        const plan = planFromDoc(grouping, (a.weed as WeedOptions | null | undefined) ?? null);
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
        const cut = passes.filter((p) => p.enabled).map((p) => plan.passes.find((q) => q.key === p.key)!);
        return syntheticTravel(cut);
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
      const plan = planFromDoc(request.grouping, request.weed ?? null);
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
      // Production's own refusal, verbatim: the worker answers a `Resume` outside
      // `WaitingForColorSwap` with `DeviceError::Busy`, which the desktop reports as
      // `device_busy` (#73). It used to read `device_error` with an invented message.
      if (status.phase !== "AwaitingColorSwap") throw ipcError("device_busy", "the cutter cannot do that right now");
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
        // Production's own words and code: `cutplan::presets::PresetError::Unreadable` through
        // `From<PresetError> for IpcError`. Invented prose here (it used to throw
        // `Corrupt("the presets file could not be read")`) meant the two assertions on this
        // banner matched a string production never sends (#278).
        throw ipcError("presets_unreadable",
          "the presets file could not be read (Permission denied (os error 13))");
      }
      const list = effectivePresets(a.machineId as string);
      if (!holdingPresets) return list;
      // Executor form, like the parked plan and travel replies above: the UI's `lib` is older than
      // `Promise.withResolvers`, and widening it for a fake is the wrong end of the trade.
      return new Promise<MaterialPreset[]>((resolve) => heldPresets.push(() => resolve(list)));
    },
    // The bounds `cutplan::preflight::SETTINGS_RANGES` publishes, restated here because a fake has
    // to answer something; the casing and the numbers are pinned on the Rust side.
    settings_ranges: () => {
      const ranges = {
        speed: { min: 1, max: 30 },
        force: { min: 1, max: 33 },
        repeatCount: { min: 1, max: 10 },
        weed: WEED_RANGES,
        weedDefaults: { margin_mm: 3, lines: "None", spacing_mm: 25, clearance_mm: 1.5 },
      };
      if (rangesFail) throw ipcError("io", "settings ranges could not be read");
      if (holdingRanges) return new Promise((resolve) => heldRanges.push(() => resolve(ranges)));
      return ranges;
    },
    // Every refusal `desktop::device::save_preset` makes, because the editor is what must never
    // send one: an entry under a builtin's pair shadows a shipped material with no way back, an
    // id-less entry is dropped on load (a save the operator never gets back), and a setting out of
    // range is refused at cut time instead. A fake more forgiving than Rust would let the editor
    // ship any of those green.
    save_preset: (a) => {
      const p = a.p as MaterialPreset;
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
      // Last, because the file is the last thing production touches: every refusal above is
      // decided without writing, so none of them may lose the race to a disk fault
      // (`what_a_preset_is_refuses_it_before_the_file_is_touched`). Production's own words and
      // code, as on the read side above.
      if (failNextPresetSave) {
        failNextPresetSave = false;
        throw ipcError("presets_unwritable",
          "the presets file could not be written (Permission denied (os error 13))");
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
      // The wiring gate (#85): a command name or payload key the Rust side does not declare is
      // refused here, rather than reaching an operator as an invalid-args error from a real
      // backend. A missing inventory is this file's own setup failing, not a call to wave through.
      // Installed by the beforeEach below, so nothing in the page's own types knows about it.
      const injected = window as unknown as { __IPC_INVENTORY__?: Record<string, string[]> };
      const inventory = injected.__IPC_INVENTORY__;
      // Refusals are recorded as well as rejected. What the frontend does with a rejection is its
      // own business — a dialog names it, a poll swallows it — and the afterEach below is what
      // makes a mis-wired call fail the test that made it, saying which command and which key
      // rather than leaving a missing element to be explained.
      //
      // In sessionStorage rather than on `window`, because a reload or a navigation gives the page
      // a fresh `window` and a refusal from before it still has to fail the test. The key is
      // spelled out at each use: this function is serialized into the page, so it can share no
      // constant with the hooks that read it.
      const refuse = (message: string) => {
        const stored: unknown = JSON.parse(sessionStorage.getItem("__ipc_violations__") ?? "[]");
        const seen = Array.isArray(stored) ? stored : [];
        sessionStorage.setItem("__ipc_violations__", JSON.stringify([...seen, message]));
        return Promise.reject(new Error(message));
      };
      if (!inventory) return refuse("ipc inventory was not installed");
      // `__test_` names are this fake's own hooks, invoked over this channel because it is the only
      // channel a test has. They have no Rust counterpart by design.
      if (!cmd.startsWith("__test_")) {
        // `inventory` carries no prototype (see the beforeEach), so a command named `toString` or
        // `constructor` is absent rather than an inherited function that passes for declared.
        const declared = inventory[cmd];
        if (!declared) return refuse(`unregistered command: ${cmd}`);
        const undeclared = Object.keys(args).filter((k) => !declared.includes(k));
        if (undeclared.length > 0) return refuse(`undeclared argument for ${cmd}: ${undeclared.join(", ")}`);
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

// The inventory travels in its own init script because `addInitScript` passes one argument and
// `opts` already has it, and because the fake is serialized into the page and so can close over
// nothing outside itself — including this import.
test.beforeEach(async ({ page }) => {
  await page.addInitScript((inventory) => {
    const injected = window as unknown as { __IPC_INVENTORY__: unknown };
    // Prototype-less: a lookup table, so `Object.prototype`'s members are not commands.
    injected.__IPC_INVENTORY__ = Object.assign(Object.create(null), inventory);
  }, ipcInventory as Record<string, string[]>);
});

// Every test's own check on the seam: a call the registered commands do not declare fails the test
// that made it, by name, whether or not the frontend showed anything for it.
test.afterEach(async ({ page }) => {
  // A test that failed before its first navigation invoked nothing, and about:blank has no storage
  // of its own to read.
  if (!page.url().startsWith("http")) return;
  const refused = await page.evaluate(() => {
    const stored: unknown = JSON.parse(sessionStorage.getItem("__ipc_violations__") ?? "[]");
    return Array.isArray(stored) ? stored : [];
  });
  expect(refused, "the fake refused an IPC call the desktop's registered commands do not declare").toEqual([]);
});

/** The gate every other test in this file now leans on. Both refusals are stated here because a
 *  gate that stopped biting would otherwise show up as nothing at all: every test would keep
 *  passing, which is the failure #85 is about. */
test("a command or key the registered commands do not declare is refused", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");

  const call = (cmd: string, args: Record<string, unknown>) =>
    page.evaluate(
      (c) => {
        // The fake's own channel, as everywhere else in this file.
        const internals = window as unknown as {
          __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
        };
        return internals.__TAURI_INTERNALS__.invoke(c.cmd, c.args).then(
          () => "resolved",
          (e: unknown) => (e instanceof Error ? e.message : String(e)),
        );
      },
      { cmd, args },
    );

  expect(await call("trace_imagee", {})).toBe("unregistered command: trace_imagee");
  // `Object.prototype`'s members are not commands: a lookup that found one would read a function
  // as a declared argument list and throw on it instead of refusing the call.
  expect(await call("toString", {})).toBe("unregistered command: toString");

  // The rename that shipped green for two commits: Rust's parameter is `controls`, and `ipc.ts`
  // kept sending `opts` (#85). A registered command is not a licence to send it anything.
  expect(await call("trace_image", { path: "/tmp/fake.png", opts: {} })).toBe(
    "undeclared argument for trace_image: opts",
  );

  // The same call as the frontend makes it, which the gate must let through — a check that refuses
  // everything is indistinguishable from one that refuses nothing.
  expect(await call("trace_image", { path: "/tmp/fake.png", controls: {} })).toBe("resolved");

  // Cleared because they were the point: the afterEach above fails any test that leaves one.
  await page.evaluate(() => sessionStorage.removeItem("__ipc_violations__"));
});

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

test("a weed border and lines replan into the preview, and the cut carries the options", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  const preview = page.getByRole("img", { name: /Cut preview/ });
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");
  // Lines wait for a border: without one their ends would stop in the open sheet.
  await expect(page.getByLabel("Weed lines")).toBeDisabled();
  await expect(page.getByLabel("Weed margin")).toBeDisabled();

  await page.getByLabel("Weed border").check();
  // One border per pass, each its own sheet, and the blade travels to each after its shapes.
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
  await expect(page.getByLabel("Weed lines")).toBeEnabled();
  await expect(page.getByLabel("Weed spacing")).toBeDisabled();

  await page.getByLabel("Weed lines").selectOption("Horizontal");
  await page.getByLabel("Weed spacing").fill("5");
  // Each 10 mm rect's border runs -3..13 on y; lines at y = 2 and 7 cross the rect and are cut off
  // within the clearance, leaving stubs under 2 mm, so only y = 12 survives: one line per pass.
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 5 travel moves, 4 weed paths");

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Waiting for color swap")).toBeVisible();
  const request = await callFake(page, "__test_last_cut_request") as { weed: unknown };
  expect(request.weed).toEqual({ margin_mm: 3, lines: "Horizontal", spacing_mm: 5, clearance_mm: 1.5 });
});

test("a weed field out of range disables Cut and says why", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await page.getByLabel("Weed border").check();
  const start = page.getByRole("button", { name: "Start Cut" });
  await expect(start).toBeEnabled();

  await page.getByLabel("Weed margin").fill("60");
  await expect(page.getByRole("alert").filter({ hasText: "Margin must be 0.5–50 mm" })).toBeVisible();
  await expect(start).toBeDisabled();
  // Not cut without its border: the last plan still holds the old margin, so Cut waits.
  await page.getByLabel("Weed margin").fill("4");
  await expect(start).toBeEnabled();

  await page.getByLabel("Weed lines").selectOption("Both");
  await page.getByLabel("Weed clearance").fill("4");
  await expect(page.getByRole("alert").filter({ hasText: "Clearance must be less than the margin" })).toBeVisible();
  await expect(start).toBeDisabled();
});

// What a replan for a weed edit does to the rows: the operator's arrangement (which passes are cut,
// in what order, with what settings) stays, since the weed changes none of the passes.
test("a weed edit keeps the passes as the operator arranged them, and their travel", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  const rows = page.getByTestId("cut-pass-row");
  await expect(rows).toHaveCount(2);
  await rows.nth(1).getByRole("button", { name: "Up" }).click(); // green first
  await rows.nth(1).getByRole("checkbox").uncheck(); // and red not cut
  const preview = page.getByRole("img", { name: /Cut preview/ });
  await expect(preview).toHaveAccessibleName("Cut preview: 1 pass, 0 travel moves");

  await page.getByLabel("Weed border").check();
  await expect(rows.nth(0).locator("span").first()).toHaveCSS("background-color", "rgb(0, 255, 0)");
  await expect(rows.nth(1).getByRole("checkbox")).not.toBeChecked();
  // Travel for the list as arranged: only green is cut, so one move, to its border. The plan's
  // own travel (both passes, in planned order) would read 3.
  await expect(preview).toHaveAccessibleName("Cut preview: 1 pass, 1 travel move, 1 weed path");
});

// Reordering asks the backend for travel again, and that request must carry the weed the plan was
// made with: without it the replanned travel drops the moves to each border.
test("reordering passes keeps the weed in the replanned travel", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByLabel("Weed border").check();
  const preview = page.getByRole("img", { name: /Cut preview/ });
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
  await page.getByTestId("cut-pass-row").nth(1).getByRole("button", { name: "Up" }).click();
  await expect(page.getByTestId("cut-pass-row").first().locator("span").first()).toHaveCSS("background-color", "rgb(0, 255, 0)");
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
});

test("Cut waits while a weed replan is out, so a border nobody has previewed cannot be cut", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  const start = page.getByRole("button", { name: "Start Cut" });
  await expect(start).toBeEnabled();
  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByLabel("Weed border").check();
  await expect(start).toBeDisabled();
  await page.evaluate(() => (window as unknown as { __releasePlans: () => Promise<void> }).__releasePlans());
  await expect(start).toBeEnabled();
});

test("a refused weed replan puts the controls back to the plan still installed", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();
  await page.getByLabel("Weed border").check();
  const preview = page.getByRole("img", { name: /Cut preview/ });
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");

  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByLabel("Weed lines").selectOption("Both");
  // The controls say what the installed plan holds, so Cut is offered for what the preview shows.
  await expect(page.getByLabel("Weed lines")).toHaveValue("None");
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
});

// A refusal arriving after the operator has typed on must not put back over what they typed.
test("a refused weed replan does not overwrite what was typed after it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByLabel("Weed border").check();
  await expect(page.getByRole("img", { name: /Cut preview/ })).toHaveAccessibleName(/2 weed paths/);
  await page.evaluate(() => {
    const w = window as unknown as { __armHold: () => void; __refuseHeldPlans: () => void };
    w.__armHold();
    w.__refuseHeldPlans();
  });
  await page.getByLabel("Weed margin").fill("5"); // replan out, held
  await page.getByLabel("Weed margin").fill(""); // mid-edit: reads as nothing, plans nothing
  await page.evaluate(() => (window as unknown as { __releasePlans: () => Promise<void> }).__releasePlans());
  await expect(page.getByLabel("Weed margin")).toHaveValue("");
});

test("changing the grouping keeps the weed", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByLabel("Weed border").check();
  await page.getByLabel("Group passes by").selectOption("Single");
  // One pass over both rects, so one border around both.
  await expect(page.getByRole("img", { name: /Cut preview/ })).toHaveAccessibleName("Cut preview: 1 pass, 1 travel move, 1 weed path");
});

test("Border waits for the weed defaults, and says why while it waits or when they fail", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.evaluate(() => (window as unknown as { __holdRanges: () => void }).__holdRanges());
  await page.getByRole("button", { name: "Cut" }).click();
  const border = page.getByLabel("Weed border");
  // Ticked now it would start from an empty margin.
  await expect(border).toBeDisabled();
  await expect(border).toHaveAttribute("title", "Waiting for the weed defaults");
  await page.evaluate(() => (window as unknown as { __releaseRanges: () => Promise<void> }).__releaseRanges());
  await border.check();
  await expect(page.getByLabel("Weed margin")).toHaveValue("3");

  await page.getByRole("button", { name: "Close" }).click();
  await page.reload();
  await page.evaluate(() => (window as unknown as { __failRanges: () => void }).__failRanges());
  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByLabel("Weed border")).toBeDisabled();
  await expect(page.getByLabel("Weed border")).toHaveAttribute("title", /Weeding is unavailable: settings ranges could not be read/);
});

test("reopening the cut dialog keeps the weed controls as they were left", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Cut" }).click();
  await page.getByLabel("Weed border").check();
  await page.getByLabel("Weed margin").fill("4");
  await page.getByRole("button", { name: "Close" }).click();

  await page.getByRole("button", { name: "Cut" }).click();
  await expect(page.getByLabel("Weed border")).toBeChecked();
  await expect(page.getByLabel("Weed margin")).toHaveValue("4");
  // And the first plan of the reopened dialog is made with them.
  await expect(page.getByRole("img", { name: /Cut preview/ })).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
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
  await expect(row).toContainText("card-stock (name unread)");
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

  // Greptile's P1 on the second push: disconnecting clears the list with the aim, and presets are
  // machine-scoped, so from here nothing can resolve the name — not "reading", which is why the
  // marker does not say so. What must not come back is `(unknown preset)`: the material is not gone,
  // and the file the backend would resolve it from is untouched by a disconnect.
  await page.getByLabel("Disconnect usb:mock").click();
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("unknown preset");
  await expect(page.getByLabel("Preset for pass 1")).toHaveValue("preset:card-stock");
});

// #274, the sharper half of the same family: the list a row prices from is replaced only by a read
// that succeeds, so between a write landing and its re-read arriving — and forever if that read
// fails — the row names and prices the pass from the entry as it was *before* the write, while
// `prepare_cut` resolves the new one from the presets file. A stale number reads as a fact.
test("a written preset's previous name and settings leave the pass row when the write lands", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedUserPreset: true,
  });
  await page.goto("/");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);
  await page.getByTestId("layer-row").first().click();
  await page.getByLabel("Material preset").selectOption("preset:card-stock");

  await openDialogOnCameo(page);
  await page.getByLabel("Group passes by").selectOption("Preset");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  const row = page.getByTestId("cut-pass-row").first();
  await expect(row).toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("1");

  // Rename the material the row is priced from and make it cut three times, with every list reply
  // parked: the write lands, so that is what the file holds and what the blade will do, and the
  // read that would say so is still out.
  await page.getByLabel("Preset to manage").selectOption("card-stock");
  await page.getByLabel("Preset name").fill("Heavy Card");
  await page.getByLabel("Preset repeat count").fill("3");
  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Save preset", { exact: true }).click();

  // Neither the name nor the repeat the entry used to have. Unread is what the row has: an answer
  // is owed and none has arrived.
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  // Cutting stays available throughout: the backend resolves the material from the presets file,
  // and refuses by name when it cannot, so nothing here is a gate.
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
  // And the editor above the rows does not go with them. It is withheld only until *some* list for
  // this aim has arrived, because what is stored is what a new entry's id and name must avoid —
  // withholding it here would take it off screen on every save.
  await expect(page.getByLabel("Preset name")).toHaveValue("Heavy Card");

  await callFake(page, "__test_release_presets");
  await expect(row).toContainText("Heavy Card");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("3");

  // The half that outlives the window: a re-read that fails never replaces the list, so the row
  // must not fall back to the entry the write superseded — which it did for the life of the dialog.
  await page.getByLabel("Preset repeat count").fill("5");
  await callFake(page, "__test_fail_next_preset_list");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByText(/Material presets are unavailable/)).toBeVisible();
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("Heavy Card");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
});

// The delete half of the same window, which is its own branch of the dialog and can regress on its
// own (Copilot on PR #275): a delete lands in the presets file exactly as a save does, so the list
// held from before it is just as superseded — and a row priced from it would name a material the
// operator has just thrown away.
test("a deleted preset's name and settings leave the pass row when the delete lands", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedUserPreset: true,
  });
  await page.goto("/");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);
  await page.getByTestId("layer-row").first().click();
  await page.getByLabel("Material preset").selectOption("preset:card-stock");

  await openDialogOnCameo(page);
  await page.getByLabel("Group passes by").selectOption("Preset");
  const row = page.getByTestId("cut-pass-row").first();
  await expect(row).toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("1");

  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Preset to manage").selectOption("card-stock");
  await page.getByLabel("Delete preset").click();

  // Unread, not "unknown preset": the delete has landed but nothing has read the file back, so the
  // row cannot yet say the material is gone — only that it has no answer about it.
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
  // The editor stays, for the same reason it stays across a save: it is what the held entries are
  // kept for.
  await expect(page.getByLabel("Preset to manage")).toBeVisible();

  // Once a list does arrive, the row may say what it now knows — the document names a preset this
  // cutter no longer has — and prices the pass at the default single pass.
  await callFake(page, "__test_release_presets");
  await expect(row).toContainText("card-stock (unknown preset)");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("1");

  // Re-mint the entry the document names, so the failing window has a name and a repeat count of
  // its own to leave behind rather than the ones already proven above.
  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card Stock");
  await page.getByLabel("Preset repeat count").fill("4");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(row).toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("4");

  // And a re-read that fails after a delete leaves the row unread for good, rather than falling
  // back to the entry the delete removed.
  await callFake(page, "__test_fail_next_preset_list");
  await page.getByLabel("Preset to manage").selectOption("card-stock");
  await page.getByLabel("Delete preset").click();
  await expect(page.getByText(/Material presets are unavailable/)).toBeVisible();
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  await expect(page.getByRole("button", { name: "Start Cut" })).toBeEnabled();
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

  // Reaches past the UI on purpose: the canvas drag that would edit the document (through
  // commit_transforms) is behind the open dialog, and the backend contract under test, geometry
  // that changes with no node added or removed, is the same either way.
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

// The canvas's pixels are unreadable from a test, so these turn document mm into page px with the
// view the canvas publishes as `data-view="scale tx ty"`.
type CanvasView = { scale: number; tx: number; ty: number };

async function readView(page: Page): Promise<CanvasView> {
  const attr = (await page.getByTestId("design-canvas").getAttribute("data-view")) ?? "";
  const [scale, tx, ty] = attr.split(" ").map(Number);
  return { scale, tx, ty };
}

async function fittedView(page: Page): Promise<CanvasView> {
  await expect(page.getByTestId("design-canvas")).not.toHaveAttribute("data-view", "1 0 0");
  return readView(page);
}

async function toPage(page: Page, v: CanvasView, mm: { x: number; y: number }) {
  const box = await page.getByTestId("design-canvas").boundingBox();
  if (!box) throw new Error("design canvas has no layout box");
  return { x: box.x + mm.x * v.scale + v.tx, y: box.y + mm.y * v.scale + v.ty };
}

/** Ctrl-wheel at (the whole pixel nearest) a document point; returns the view once the zoom has
 *  landed. Whole pixels because Chromium reports a wheel's position as integers: a fractional
 *  target puts the pinned point up to a pixel away, and a 20× zoom turns that into twenty. */
async function zoomInAt(page: Page, mm: { x: number; y: number }, deltaY: number): Promise<CanvasView> {
  const before = await fittedView(page);
  const at = await toPage(page, before, mm);
  await page.mouse.move(Math.round(at.x), Math.round(at.y));
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, deltaY);
  await page.keyboard.up("Control");
  await expect.poll(async () => (await readView(page)).scale).toBeGreaterThan(before.scale);
  return readView(page);
}

test("Ctrl-wheel zooms about the cursor and the status bar reports it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v0 = await fittedView(page);
  const zoom = page.getByTestId("status-zoom");
  const before = parseInt((await zoom.textContent()) ?? "", 10);
  const target = await toPage(page, v0, { x: 5, y: 5 });
  const cursor = { x: Math.round(target.x), y: Math.round(target.y) }; // where zoomInAt puts it
  const box = (await page.getByTestId("design-canvas").boundingBox())!;
  const under = { x: (cursor.x - box.x - v0.tx) / v0.scale, y: (cursor.y - box.y - v0.ty) / v0.scale };

  const v1 = await zoomInAt(page, { x: 5, y: 5 }, -300);

  await expect.poll(async () => parseInt((await zoom.textContent()) ?? "", 10)).toBeGreaterThan(before);
  // About the cursor: the document point that was under it is still under it.
  const after = await toPage(page, v1, under);
  expect(after.x).toBeCloseTo(cursor.x, 0);
  expect(after.y).toBeCloseTo(cursor.y, 0);
});

test("dragging a corner handle commits one scale about the opposite corner", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click(); // the red 10 × 10 mm rect at the origin
  // Far enough in that the handles are tens of px apart.
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  const se = await toPage(page, v, { x: 10, y: 10 });
  await page.mouse.move(se.x, se.y);
  await page.mouse.down();
  await page.mouse.move(se.x + 10 * v.scale, se.y + 5 * v.scale, { steps: 4 });
  await page.mouse.up();

  const commits = await page.evaluate(
    () => (window as unknown as { __commitTransforms?: { ids: number[]; m: number[] }[] }).__commitTransforms ?? [],
  );
  expect(commits).toHaveLength(1); // one gesture, one commit, one undo entry
  expect(commits[0].ids).toEqual([2]);
  // 10 → 20 mm wide, 10 → 15 mm tall, with the nw corner at the origin held still.
  const [a, b, c, d, e, f] = commits[0].m;
  expect(a).toBeCloseTo(2, 1);
  expect(d).toBeCloseTo(1.5, 1);
  for (const zero of [b, c, e, f]) expect(zero).toBeCloseTo(0, 1);
});

test("a marquee over both shapes selects both", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  // Zoomed in so the band's start point is clear of the shapes' hit tolerance. The band runs from
  // empty space below-right up to a point inside both shapes: the shapes sit at the artboard's top
  // edge, so after zooming there is almost no canvas above them, and a band only has to touch.
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  const from = await toPage(page, v, { x: 20, y: 20 });
  const to = await toPage(page, v, { x: 4, y: 4 });
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 5 });
  await page.mouse.up();

  await expect(page.locator('[data-testid="layer-row"][data-selected="true"]')).toHaveCount(2);
});

test("the cursor readout follows a wheel pan under a still pointer", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v0 = await fittedView(page);
  const box = (await page.getByTestId("design-canvas").boundingBox())!;
  const pointer = { x: Math.round(box.x + box.width / 2), y: Math.round(box.y + box.height / 2) };
  await page.mouse.move(pointer.x, pointer.y);
  const readY = async () => Number(/y (-?[\d.]+) mm/.exec((await page.getByTestId("status-cursor").textContent()) ?? "")?.[1]);
  const before = await readY();

  await page.mouse.wheel(0, 100); // scroll down: the view pans up, the pointer is over a lower point
  await expect.poll(async () => (await readView(page)).ty).toBeLessThan(v0.ty);

  // A readout stored at the last pointer move would still say `before`; the world moved under it.
  await expect.poll(readY).toBeCloseTo(before + 100 / v0.scale, 0);
});

// One transform on the wire at a time. A second gesture pressed while the first commit is
// unanswered would compute its matrix from the first one's preview; if the first were then
// refused, the second would land about the wrong anchor (CodeRabbit on #298).
test("a drag pressed while the previous commit is in flight does not commit", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const commits = () =>
    page.evaluate(() => ((window as unknown as { __commitTransforms?: unknown[] }).__commitTransforms ?? []).length);
  const drag = async (from: { x: number; y: number }, dx: number) => {
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(from.x + dx, from.y, { steps: 3 });
    await page.mouse.up();
  };

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  const centre = await toPage(page, v, { x: 5, y: 5 });
  await drag(centre, 5 * v.scale); // move right 5 mm; its commit is now parked
  await expect.poll(commits).toBe(1);

  await drag({ x: centre.x + 5 * v.scale, y: centre.y }, 5 * v.scale); // press on the preview
  expect(await commits()).toBe(1);

  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
  await drag({ x: centre.x + 5 * v.scale, y: centre.y }, 5 * v.scale); // settled: drags commit again
  await expect.poll(commits).toBe(2);
});

type CommitRecord = { ids: number[]; m: number[]; batch?: number };

async function commitLog(page: Page): Promise<CommitRecord[]> {
  return page.evaluate(() => (window as unknown as { __commitTransforms?: CommitRecord[] }).__commitTransforms ?? []);
}

async function dragBy(page: Page, from: { x: number; y: number }, dx: number, dy: number) {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + dx, from.y + dy, { steps: 4 });
  await page.mouse.up();
}

/** The fake's own transform for a node, read through the same command the app uses. */
async function nodeTransform(page: Page, id: number): Promise<number[]> {
  const json = await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("snapshot", {}),
  );
  return (JSON.parse(json as string) as { nodes: Record<string, { transform: number[] }> }).nodes[id].transform;
}

test("a refused transform puts the shape back and the next drag starts from where it was", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const centre = await toPage(page, v, { x: 5, y: 5 });

  await page.evaluate(() => (window as unknown as { __failNextCommit: () => void }).__failNextCommit());
  await dragBy(page, centre, 5 * v.scale, 0);
  await expect(page.getByText("transform refused")).toBeVisible();

  // Pressed where the shape was: a preview left stranded would put the box 5 mm to the right.
  await dragBy(page, centre, 5 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(5, 0);
});

test("a transform that lands but cannot be re-read keeps the shape where the backend has it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const centre = await toPage(page, v, { x: 5, y: 5 });

  await page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await dragBy(page, centre, 20 * v.scale, 0); // the commit lands; its refresh fails
  await expect(page.getByText(/Edit applied, but the canvas could not be refreshed/)).toBeVisible();

  // The box is where the backend put it (20 mm right), clear of where the shape used to be: a
  // revert to the stale scene would make this press miss and start a marquee instead.
  await dragBy(page, { x: centre.x + 20 * v.scale, y: centre.y }, 5 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(25, 0);
});

test("an edit keeps the operator's zoom; Ctrl+0 and a machine switch refit", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const fitted = await fittedView(page);
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  // The snapshot after the commit rebuilds the artboard object; the view must not refit for it.
  expect(await readView(page)).toEqual(v);

  await page.getByTestId("design-canvas").hover();
  await page.keyboard.press("Control+0");
  await expect.poll(() => readView(page)).toEqual(fitted);

  await page.getByLabel("Machine").selectOption("puma");
  const box = (await page.getByTestId("design-canvas").boundingBox())!;
  // The Puma's 600 × 5000 mm bed, height-bound in this window like the Cameo's.
  await expect.poll(async () => (await readView(page)).scale).toBeCloseTo((box.height - 48) / 5000, 3);
});

test("keyboard zoom works after a toolbar click and leaves a focused field alone", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const fitted = await fittedView(page);
  await page.getByRole("button", { name: "Select" }).click(); // focus stays on the button

  await page.keyboard.press("Control+=");
  await expect.poll(async () => (await readView(page)).scale).toBeCloseTo(fitted.scale * 1.25, 6);
  await page.keyboard.press("Control+1");
  await expect(page.getByTestId("status-zoom")).toHaveText("100%");
  await page.keyboard.press("Control+0");
  await expect.poll(() => readView(page)).toEqual(fitted);

  await page.getByTestId("layer-row").first().click();
  await page.getByLabel("X", { exact: true }).focus();
  await page.keyboard.press("Control+=");
  expect(await readView(page)).toEqual(fitted);
});

test("Shift-click inside the selection toggles a shape out without dragging the rest", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  await dragBy(page, await toPage(page, v, { x: 20, y: 20 }), -16 * v.scale, -16 * v.scale); // marquee both
  const selectedRows = page.locator('[data-testid="layer-row"][data-selected="true"]');
  await expect(selectedRows).toHaveCount(2);

  // Both seeded rects share the origin, so this lands on the topmost of the two.
  await page.keyboard.down("Shift");
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0);
  await page.keyboard.up("Shift");

  await expect(selectedRows).toHaveCount(1);
  expect(await commitLog(page)).toEqual([]);
});

test("Space-drag pans after a toolbar click without pressing the button, and so does a middle drag", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v0 = await fittedView(page);
  await page.getByRole("button", { name: "Rectangle" }).click(); // adds one; focus stays on it
  const rows = page.getByTestId("layer-row");
  await expect(rows).toHaveCount(3);
  const box = (await page.getByTestId("design-canvas").boundingBox())!;
  const start = { x: Math.round(box.x + box.width / 2), y: Math.round(box.y + box.height / 2) };

  await page.mouse.move(start.x, start.y);
  await page.keyboard.down("Space");
  await dragBy(page, start, 50, 30);
  await page.keyboard.up("Space");
  await expect.poll(async () => (await readView(page)).tx).toBeCloseTo(v0.tx + 50, 0);
  expect((await readView(page)).ty).toBeCloseTo(v0.ty + 30, 0);
  await expect(rows).toHaveCount(3); // Space never reached the Rectangle button
  expect(await commitLog(page)).toEqual([]);

  await page.mouse.move(start.x, start.y);
  await page.mouse.down({ button: "middle" });
  await page.mouse.move(start.x - 20, start.y, { steps: 3 });
  await page.mouse.up({ button: "middle" });
  await expect.poll(async () => (await readView(page)).tx).toBeCloseTo(v0.tx + 30, 0);
});

test("dragging just outside a corner rotates about the centre; Alt scales about it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const c = await toPage(page, v, { x: 5, y: 5 });
  const se = await toPage(page, v, { x: 10, y: 10 });

  // Inside the rotate zone (18 px) and outside the handle (6 px), a quarter turn clockwise.
  const from = { x: se.x + 8, y: se.y + 8 };
  const r = Math.hypot(from.x - c.x, from.y - c.y);
  const to = { x: c.x + r * Math.cos((3 * Math.PI) / 4), y: c.y + r * Math.sin((3 * Math.PI) / 4) };
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 8 });
  await page.mouse.up();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [a, b, cc, d, e, f] = (await commitLog(page))[0].m;
  expect([a, b, cc, d]).toEqual([expect.closeTo(0, 1), expect.closeTo(1, 1), expect.closeTo(-1, 1), expect.closeTo(0, 1)]);
  // About the centre (5, 5): it maps to itself.
  expect(a * 5 + cc * 5 + e).toBeCloseTo(5, 0);
  expect(b * 5 + d * 5 + f).toBeCloseTo(5, 0);
});

test("Alt-dragging a corner scales about the centre", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const se = await toPage(page, v, { x: 10, y: 10 });

  await page.keyboard.down("Alt");
  await dragBy(page, se, 10 * v.scale, 5 * v.scale);
  await page.keyboard.up("Alt");

  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  // Half-widths 5 → 15 and 5 → 10 about (5, 5): ×3 and ×2 with the centre fixed.
  const [a, b, c, d, e, f] = (await commitLog(page))[0].m;
  expect([a, b, c, d, e, f]).toEqual([3, 0, 0, 2, -10, -5].map((x) => expect.closeTo(x, 1)));
});

// The gesture follows the document point under the pointer, not the pointer's last world position:
// a pan under a held, still pointer must move the shape with it (CodeRabbit on #298).
test("a wheel pan during a drag carries the shape with the pointer and commits that", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const centre = await toPage(page, v, { x: 5, y: 5 });

  await page.mouse.move(centre.x, centre.y);
  await page.mouse.down();
  await page.mouse.move(centre.x + 5 * v.scale, centre.y, { steps: 3 });
  await page.mouse.wheel(0, 100); // the view pans up 100 px; the pointer now sits 100 px lower in mm
  await expect.poll(async () => (await readView(page)).ty).toBeLessThan(v.ty);
  await page.mouse.up();

  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [, , , , e, f] = (await commitLog(page))[0].m;
  expect(e).toBeCloseTo(5, 0);
  expect(f).toBeCloseTo(100 / v.scale, 0);
});

test("a Group selected in the layers panel gets a box around its shapes and commits as itself", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedGroup: true });
  await page.goto("/");
  await page.getByTestId("layer-row").filter({ hasText: "Group" }).click();
  const v = await zoomInAt(page, { x: 35, y: 5 }, -350);

  // The box spans the Group's rect, 30..40 mm; drag its se corner out by 10 × 5 mm.
  await dragBy(page, await toPage(page, v, { x: 40, y: 10 }), 10 * v.scale, 5 * v.scale);

  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([1 + 1]); // the Group (id 2, after the root), not its rect
  // Width 10 → 20 and height 10 → 15 about the nw corner (30, 0).
  const [a, b, c, d, e, f] = m;
  expect([a, b, c, d, e, f]).toEqual([2, 0, 0, 1.5, -30, 0].map((x) => expect.closeTo(x, 1)));
});

// The X/Y/W/H fields are a transform producer too: they must compute from the geometry the canvas
// shows (an unread commit's preview included) and wait their turn behind a commit on the wire, or
// they send a matrix built on the position the shape has already left (Copilot on #298).
test("a property edit after an applied-but-unread commit starts from where the shape now is", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 20 * v.scale, 0); // lands at x = 20; refresh fails
  await expect(page.getByText(/Edit applied, but the canvas could not be refreshed/)).toBeVisible();

  await page.getByLabel("X", { exact: true }).fill("5");
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1].m[4]).toBeCloseTo(-15, 1);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(5, 1);
});

test("a property edit made while a commit is in flight waits for it, then lands where it says", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 20 * v.scale, 0); // parked on the wire
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await page.getByLabel("X", { exact: true }).fill("5");
  expect((await commitLog(page)).length).toBe(1); // queued, not sent alongside

  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(5, 1);
});

test("edits to two different fields made during an in-flight commit both land", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 20 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  // Different fields do not supersede each other: a single queue slot dropped X when Y arrived
  // (CodeRabbit and Copilot on #298).
  await page.getByLabel("X", { exact: true }).fill("5");
  await page.getByLabel("Y", { exact: true }).fill("7");
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  await expect.poll(async () => (await nodeTransform(page, 2)).slice(4)).toEqual([expect.closeTo(5, 1), expect.closeTo(7, 1)]);
});

// The fake has to mirror transform_nodes for nested nodes (Copilot on #298): a scale committed on
// a Group's child lands in the parent's space, not as if the child sat at the root.
test("scaling a shape inside a moved Group keeps it where the real backend would", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedGroup: true });
  await page.goto("/");
  await page.getByTestId("layer-row").filter({ hasText: "Rectangle" }).click(); // the Group's rect
  const v = await zoomInAt(page, { x: 35, y: 5 }, -350);

  await dragBy(page, await toPage(page, v, { x: 40, y: 10 }), 10 * v.scale, 5 * v.scale);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  // World: ×2, ×1.5 about (30, 0). In the Group's space, which is translated by 30, that is a pure
  // scale about its origin.
  await expect.poll(async () => nodeTransform(page, 3)).toEqual([2, 0, 0, 1.5, 0, 0].map((x) => expect.closeTo(x, 1)));
});

// Queued edits drain one per settled commit, each built from the geometry the one before it left:
// W scales about where X put the shape, not where the drag did (Copilot on #298). They drain after
// the snapshot before them has rendered, so this no longer shows a snapshot rendering while a commit
// is on the wire; the test after it does.
test("queued X then W behind a move scale about where X put the shape", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 20 * v.scale, 0); // to x = 20, parked
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  await page.getByLabel("X", { exact: true }).fill("5");
  await page.getByLabel("W", { exact: true }).fill("20");
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The 10 mm rect ends at x = 5, 20 mm wide: scale ×2 about x = 5, so e = 5 and a = 2.
  await expect.poll(async () => (await commitLog(page)).length).toBe(3);
  await expect.poll(async () => {
    const [a, , , , e] = await nodeTransform(page, 2);
    return [a, e];
  }).toEqual([expect.closeTo(2, 1), expect.closeTo(5, 1)]);
});

// A snapshot can render while a commit is on the wire, one the backend answered before the commit
// arrived: a run() command's refresh. It must not retire that commit's preview, or the canvas and
// fields jump back and the next edit is built from geometry the backend has left (Copilot on #298).
// The fake runs a held commit on release, so the machine switch's snapshot here is that older one.
test("a snapshot rendered while a commit is on the wire keeps that commit's preview", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click(); // red, at 0
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await page.getByLabel("X", { exact: true }).fill("20"); // on the wire
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  const before = (await readView(page)).scale;
  await page.getByLabel("Machine").selectOption("puma"); // its refresh renders a new document now
  await expect.poll(async () => (await readView(page)).scale).not.toBeCloseTo(before, 6);
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("20");
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(20, 6);
});

// Snapping (spec 2026-10-07). With both seeds: the Group's rect at 30..40 mm (id 3) and the red
// rect (id 4) and green rect at 0..10 mm. Layer rows run Group, its rect, red, green.
async function selectRedBesideGroup(page: Page): Promise<CanvasView> {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await page.getByTestId("layer-row").nth(2).click();
  return zoomInAt(page, { x: 20, y: 5 }, -350);
}

test("a move that stops just short of another shape snaps flush against it", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  // Right edge would stop at 29.6 mm, 0.4 mm short of the Group's rect: within reach, so it butts.
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 19.6 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([4]);
  expect(m[4]).toBeCloseTo(20, 2);
});

test("holding Ctrl drags without snapping", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.keyboard.down("Control");
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 19.6 * v.scale, 0);
  await page.keyboard.up("Control");
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0].m[4]).toBeCloseTo(19.6, 1);
});

test("an edge handle that stops just short of another shape snaps its edge to it", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await dragBy(page, await toPage(page, v, { x: 10, y: 5 }), 19.6 * v.scale, 0); // the e handle
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [a, , , , e] = (await commitLog(page))[0].m;
  // The right edge goes 10 → 30 mm (snapped from 29.6), so the 10 mm rect becomes 30 wide: ×3, not
  // the 2.96 an unsnapped drag would give, with its left edge fixed at 0.
  expect(a).toBeCloseTo(3, 2);
  expect(e).toBeCloseTo(0, 2);
});

test("a selected Group does not snap to its own shape", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await page.getByTestId("layer-row").nth(0).click(); // the Group; its rect is excluded from targets
  const v = await zoomInAt(page, { x: 35, y: 5 }, -350);
  // 0.3 mm is well within reach of where the Group's rect started; a self-snap would pull it back.
  await dragBy(page, await toPage(page, v, { x: 35, y: 5 }), 0.3 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0].m[4]).toBeCloseTo(0.3, 1);
});

test("pressing on an unselected shape and dragging snaps too", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  const v = await zoomInAt(page, { x: 20, y: 5 }, -350);
  // Nothing selected: the press picks the topmost rect at the origin (green, id 5) and drags it.
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 19.6 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([5]);
  expect(m[4]).toBeCloseTo(20, 2);
});

test("a move can snap to the artboard's centre line", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await page.getByTestId("layer-row").nth(0).click(); // the Group, centred at x = 35
  const v = await zoomInAt(page, { x: 35, y: 5 }, -200); // ~1.5 px/mm, so the reach is ~4 mm
  // Its centre would stop at 164.6 mm; the Cameo's 330 mm bed is centred at 165, a line only the
  // artboard has.
  await dragBy(page, await toPage(page, v, { x: 35, y: 5 }), 129.6 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0].m[4]).toBeCloseTo(130, 1);
});

test("an Alt edge scale snaps the dragged edge and mirrors the other", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.keyboard.down("Alt");
  await dragBy(page, await toPage(page, v, { x: 10, y: 5 }), 19.6 * v.scale, 0); // e handle
  await page.keyboard.up("Alt");
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  // About the centre (5): the right edge snaps 29.6 → 30, so the half-width 5 becomes 25 (×5), and
  // the left edge mirrors to −20.
  const [a, , , , e] = (await commitLog(page))[0].m;
  expect(a).toBeCloseTo(5, 2);
  expect(e).toBeCloseTo(-20, 1);
});

// Align and distribute (spec 2026-10-07). With both seeds: the Group (id 2) holds a rect at 30..40
// mm, and the red (id 4) and green (id 5) rects sit at 0..10 mm. Layer rows run Group, its rect,
// red, green.
async function selectRows(page: Page, rows: number[]) {
  const [first, ...rest] = rows;
  await page.getByTestId("layer-row").nth(first).click();
  for (const r of rest) await page.getByTestId("layer-row").nth(r).click({ modifiers: ["Shift"] });
}

test("align left on two rects and a Group commits one batch that moves only the Group", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [2, 3, 0]);
  await page.getByRole("button", { name: "Align left edges" }).click();

  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m, batch }] = await commitLog(page);
  expect(ids).toEqual([2]);
  expect(m).toEqual([1, 0, 0, 1, -30, 0]);
  expect(batch).toBe(1);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(0, 6);
});

test("align on a single rect centres it on the artboard", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [2]);
  await page.getByRole("button", { name: "Align horizontal centres" }).click();

  // The 330 mm bed is centred at 165; the rect's centre is at 5.
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([4]);
  expect(m[4]).toBeCloseTo(160, 6);
  expect(m[5]).toBe(0);
});

test("distribute equalises the gaps and breaks a tie by document order, not click order", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [0, 3, 2]); // Group, green, red: the reverse of document order
  await page.getByRole("button", { name: "Distribute horizontal spacing" }).click();

  // Span 0..40 holding 30 mm of shapes leaves two 5 mm gaps. Red and green both start at 0, so
  // document order makes red first and green the one that moves, to 15.
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([5]);
  expect(m[4]).toBeCloseTo(15, 6);
});

test("align and distribute are disabled when they cannot act", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  const alignLeft = page.getByRole("button", { name: "Align left edges" });
  const distribute = page.getByRole("button", { name: "Distribute horizontal spacing" });
  await expect(alignLeft).toBeDisabled();
  await expect(distribute).toBeDisabled();

  // The Group, its own rect and red are two units, since the rect moves with the Group: still too
  // few to distribute.
  await selectRows(page, [0, 1, 2]);
  await expect(alignLeft).toBeEnabled();
  await expect(distribute).toBeDisabled();

  await page.getByTestId("layer-row").nth(3).click({ modifiers: ["Shift"] });
  await expect(distribute).toBeEnabled();
});

test("an align clicked while a drag's commit is on the wire lines up from where the drag left", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "Align left edges" }).click();
  expect((await commitLog(page)).length).toBe(1); // queued behind the drag, not sent
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // From the stale scene the Group would go to 0 (−30); from the drag's result it goes to 5.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  const { ids, m } = (await commitLog(page))[1];
  expect(ids).toEqual([2]);
  expect(m[4]).toBeCloseTo(-25, 6);
});

// With seedAlignExtras beside the rects: red (id 2) and green (id 3) at 0..10, the two-rect Group
// (id 4) spanning 50..80 × 20..50, and the empty Group (id 7). Rows run red, green, Group, its two
// rects, the empty Group.
async function seedAlignExtras(page: Page) {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedAlignExtras: true });
  await page.goto("/");
}

test("a Group aligns by the bounds of all its shapes, on both axes", async ({ page }) => {
  await seedAlignExtras(page);
  await selectRows(page, [0, 2]);

  // The Group's right edge is its second rect's, at 80; its first rect alone would say 60.
  await page.getByRole("button", { name: "Align right edges" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0]).toMatchObject({ ids: [2], m: [1, 0, 0, 1, 70, 0] });
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(70, 6);

  // Its bottom is 50, again from the second rect.
  await page.getByRole("button", { name: "Align bottom edges" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1]).toMatchObject({ ids: [2], m: [1, 0, 0, 1, 0, 40] });
});

test("distribute vertical spacing moves the middle unit on y only", async ({ page }) => {
  await seedAlignExtras(page);
  await selectRows(page, [0, 1, 2]);
  // Red and green at 0..10, the Group at 20..50: span 0..50 holds 50 mm, so the gaps are 0 and
  // green, second in document order, goes to 10.
  await page.getByRole("button", { name: "Distribute vertical spacing" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0]).toMatchObject({ ids: [3], m: [1, 0, 0, 1, 0, 10] });
});

test("an empty Group is not a unit, so it does not enable distribute", async ({ page }) => {
  await seedAlignExtras(page);
  await selectRows(page, [0, 1, 5]);
  await expect(page.getByRole("button", { name: "Distribute horizontal spacing" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Distribute vertical spacing" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Align left edges" })).toBeEnabled();
});

test("a refused align puts every unit back and the next click starts from there", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [2, 0]); // red and the Group: centres 5 and 35, so both move to 20
  await page.evaluate(() => (window as unknown as { __failNextCommit: () => void }).__failNextCommit());
  await page.getByRole("button", { name: "Align horizontal centres" }).click();
  await expect(page.getByText("transform refused")).toBeVisible();

  // A preview left in place would have both centred already, and this click would send nothing.
  await page.getByRole("button", { name: "Align horizontal centres" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  const log = await commitLog(page);
  expect(log.map((e) => [e.ids[0], e.m[4], e.batch])).toEqual([[2, -15, 1], [4, 15, 1]]); // document order
});

test("the fake refuses a whole batch that names a missing node", async ({ page }) => {
  // The fake is what the frontend is tested against, so it must not half-apply what the backend
  // refuses outright.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  const refused = await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("commit_transforms", { moves: [
        { ids: [4], m: [1, 0, 0, 1, 5, 0] },
        { ids: [999], m: [1, 0, 0, 1, 1, 0] },
      ] }).then(() => false, () => true),
  );
  expect(refused).toBe(true);
  expect(await commitLog(page)).toEqual([]);
  expect(await nodeTransform(page, 4)).toEqual([1, 0, 0, 1, 0, 0]);
});

test("queued aligns replace each other per axis, not across axes", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 5 * v.scale); // red to 5..15 × 5..15
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "Align left edges" }).click();
  await page.getByRole("button", { name: "Align top edges" }).click();
  await page.getByRole("button", { name: "Align right edges" }).click(); // replaces left, not top
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // Top takes red up to the Group's 0; right takes red to the Group's 40. Left (the Group to 5)
  // never runs.
  await expect.poll(async () => (await commitLog(page)).length).toBe(3);
  const [, top, right] = await commitLog(page);
  expect(top).toMatchObject({ ids: [4], m: [1, 0, 0, 1, 0, expect.closeTo(-5, 6)] });
  expect(right).toMatchObject({ ids: [4], m: [1, 0, 0, 1, expect.closeTo(25, 6), 0] });
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 30, 0]);
});

test("an align that would move nothing sends nothing and does not hold the next one", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [2, 3]); // red and green, both already at 0
  await page.getByRole("button", { name: "Align left edges" }).click();
  // Never invoked, not just never logged: the fake refuses an empty batch before it logs anything,
  // so an empty send would pass the log check below and put a refusal on screen.
  expect(await page.evaluate(() => (window as unknown as { __maxInFlightCommits?: number }).__maxInFlightCommits)).toBeUndefined();
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "Align left edges" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0]).toMatchObject({ ids: [2], batch: 1 });
});

test("a queued distribute does not replace a queued align on the same axis", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await page.getByTestId("layer-row").nth(3).click({ modifiers: ["Shift"] }); // green
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] }); // the Group
  await page.getByRole("button", { name: "Distribute horizontal spacing" }).click();
  await page.getByRole("button", { name: "Align left edges" }).click();
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // Distribute first: green 0..10 and the Group 30..40 stay, red goes to 15 (+10). Then align left
  // from there: red −15, the Group −30. Align alone would have sent red −5.
  await expect.poll(async () => (await commitLog(page)).length).toBe(4);
  const log = await commitLog(page);
  expect(log.slice(1).map((e) => [e.ids[0], Math.round(e.m[4] * 1e6) / 1e6, e.batch])).toEqual([[4, 10, 2], [2, -30, 3], [4, -15, 3]]);
});

test("an align's preview moves every unit while its commit is on the wire", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [0, 2, 3]); // the Group, red, green: the centre of 0..40 is 20
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await page.getByRole("button", { name: "Align horizontal centres" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(3);

  // The fields read the effective scene, so they show the preview: red and green both at 15. A
  // preview that applied only the first move (the Group's) would leave them at 0.
  await page.getByTestId("layer-row").nth(2).click();
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("15");
  await page.getByTestId("layer-row").nth(3).click();
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("15");
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
});

test("distribute spaces the pieces inside a border selected with them", async ({ page }) => {
  await seedAlignExtras(page);
  const horizontal = page.getByRole("button", { name: "Distribute horizontal spacing" });

  // Widen red to 100 mm, a border across x over green (0..10) and the Group (50..80).
  await selectRows(page, [0]);
  await page.getByLabel("W", { exact: true }).fill("100");
  await expect.poll(async () => (await nodeTransform(page, 2))[0]).toBeCloseTo(10, 6);

  await selectRows(page, [0, 2]);
  await expect(horizontal).toBeDisabled();
  await expect(horizontal).toHaveAttribute("title", /select three or more pieces/);

  // 100 mm less 40 mm of pieces leaves 60 over three spaces: green to 20, the Group already at 50.
  await page.getByTestId("layer-row").nth(1).click({ modifiers: ["Shift"] });
  await horizontal.click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(2); // the W edit, then this
  expect((await commitLog(page))[1]).toMatchObject({ ids: [3], m: [1, 0, 0, 1, 20, 0] });
});

test("a border too small for its pieces blocks distribute on that axis only, and says why", async ({ page }) => {
  await seedAlignExtras(page);
  const horizontal = page.getByRole("button", { name: "Distribute horizontal spacing" });
  const vertical = page.getByRole("button", { name: "Distribute vertical spacing" });

  // Red becomes a 100 mm border; green grows to 75 mm, so with the 30 mm Group the pieces need
  // 105 mm across. Down y nothing spans and the three distribute between two ends as usual.
  await selectRows(page, [0]);
  await page.getByLabel("W", { exact: true }).fill("100");
  await expect.poll(async () => (await nodeTransform(page, 2))[0]).toBeCloseTo(10, 6);
  await selectRows(page, [1]);
  await page.getByLabel("W", { exact: true }).fill("75");
  await expect.poll(async () => (await nodeTransform(page, 3))[0]).toBeCloseTo(7.5, 6);

  await selectRows(page, [0, 1, 2]);
  await expect(horizontal).toBeDisabled();
  await expect(horizontal).toHaveAttribute("title", /the pieces do not fit inside the one around them/);
  await expect(vertical).toBeEnabled();
  await expect(vertical).toHaveAttribute("title", "Distribute vertical spacing");
});

// #298: a commit drained from the queue was made before the refusal ahead of it arrived, so it
// must not clear that refusal's message before the operator has seen it.
test("a refusal stays on screen when an edit queued behind it lands", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => {
    const w = window as unknown as { __holdCommits: () => void; __failNextCommit: () => void };
    w.__holdCommits();
    w.__failNextCommit();
  });
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // will be refused
  await page.getByLabel("X", { exact: true }).fill("30"); // queued behind it
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(30, 6);
  await expect(page.getByText("transform refused")).toBeVisible();
});

test("a queued edit whose refresh succeeds clears the refresh warning ahead of it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // lands, refresh fails
  await page.getByLabel("X", { exact: true }).fill("30"); // queued behind it
  await page.evaluate(() => {
    const w = window as unknown as { __failNextSnapshot: () => void; __releaseCommits: () => Promise<void> };
    w.__failNextSnapshot();
    return w.__releaseCommits();
  });

  // The warning was true when it went up; once the queued edit has re-read the canvas it is not.
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(30, 6);
  await expect(page.getByText("the canvas could not be refreshed")).toBeHidden();
});

test("a fresh edit after a refusal clears its message", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __failNextCommit: () => void }).__failNextCommit());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0);
  await expect(page.getByText("transform refused")).toBeVisible();

  // Made after the refusal was on screen, so it is the operator's next act and starts clean.
  await page.getByLabel("X", { exact: true }).fill("30");
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(30, 6);
  await expect(page.getByText("transform refused")).toBeHidden();
});

// Copilot on #301: going out from the queue is not the same as being made before the refusal. An
// align clicked once the refusal is on screen, while an edit queued earlier is still on the wire,
// is the operator's next act and must clear it when it lands.
test("an edit made after a refusal was shown clears it, even when it waits in the queue", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => {
    const w = window as unknown as { __holdCommits: () => void; __failNextCommit: () => void };
    w.__holdCommits();
    w.__failNextCommit();
  });
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // will be refused
  await page.getByLabel("X", { exact: true }).fill("30"); // queued before the refusal
  // Let the refusal through and hold again at once, so the X edit it drains stays on the wire.
  await page.evaluate(() => {
    const w = window as unknown as { __holdCommits: () => void; __releaseCommits: () => Promise<void> };
    void w.__releaseCommits();
    w.__holdCommits();
  });
  await expect(page.getByText("transform refused")).toBeVisible();

  await page.getByRole("button", { name: "Align horizontal centres" }).click(); // queued behind X
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(160, 6);
  await expect(page.getByText("transform refused")).toBeHidden();
});

test("a newer align replaces a queued one on the same selection in any click order", async ({ page }) => {
  // Copilot on #301: the key followed click order, so reselecting a piece made a second request
  // that ran after the first instead of replacing it.
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] }); // [red, Group]
  await page.getByRole("button", { name: "Align left edges" }).click();
  await page.getByTestId("layer-row").nth(2).click({ modifiers: ["Shift"] }); // [Group]
  await page.getByTestId("layer-row").nth(2).click({ modifiers: ["Shift"] }); // [Group, red]
  await page.getByRole("button", { name: "Align right edges" }).click();
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // Right alone: red to the Group's 40 (+25). Left first would have moved the Group to 5.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1]).toMatchObject({ ids: [4], m: [1, 0, 0, 1, expect.closeTo(25, 6), 0] });
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 30, 0]);
});

test("a queued align to the artboard uses the bed it lands on, not the one it was clicked on", async ({ page }) => {
  // Copilot on #301: the queued click kept the render's artboard, so a machine switch in between
  // centred the piece on the old bed.
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  await page.getByRole("button", { name: "Align horizontal centres" }).click(); // queued

  const before = (await readView(page)).scale;
  await page.getByLabel("Machine").selectOption("puma"); // 600 mm bed
  await expect.poll(async () => (await readView(page)).scale).not.toBeCloseTo(before, 6);
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // Red's centre is 10; the Puma's is 300. The Cameo's 165 would give 155.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1]).toMatchObject({ ids: [4], m: [1, 0, 0, 1, expect.closeTo(290, 6), 0] });
});

test("the align row follows the preview while its commit is on the wire", async ({ page }) => {
  // Copilot on #301: the preview lived in a ref, so the buttons kept the bounds from before it.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [0, 2, 3]); // the Group, red, green: all 10 mm wide
  const horizontal = page.getByRole("button", { name: "Distribute horizontal spacing" });
  await expect(horizontal).toBeEnabled();

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await page.getByRole("button", { name: "Align left edges" }).click();
  // In the preview all three sit at 0..10, so each spans the rest and distribute cannot act.
  await expect(horizontal).toBeDisabled();
  await expect(horizontal).toHaveAttribute("title", /spans the selection on this axis/);
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
});

test("the fake refuses a whole batch when a later entry's geometry cannot be reversed", async ({ page }) => {
  // Copilot on #301: the first entry is fine and the second sits under a Group scaled to nothing.
  // Rust refuses the batch and keeps neither; the fake must too, or a test can pass on a half-commit.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedCollapsedGroup: true });
  await page.goto("/");
  const refused = await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("commit_transforms", { moves: [
        { ids: [2], m: [1, 0, 0, 1, 5, 0] },
        { ids: [5], m: [1, 0, 0, 1, 1, 0] },
      ] }).then(() => false, () => true),
  );
  expect(refused).toBe(true);
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 0, 0]);
  expect(await nodeTransform(page, 5)).toEqual([1, 0, 0, 1, 0, 0]);
  // Nor logged: a refused batch that took a batch number would shift every later one.
  expect(await commitLog(page)).toEqual([]);
});

test("the fake accepts a batch under a tiny but invertible parent, as Rust does", async ({ page }) => {
  // Copilot on #301: Rust refuses only an exactly zero determinant, so a 1e-7 scale (determinant
  // 1e-14) must commit; a tolerance in the fake refused it and could fail a valid test.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedCollapsedGroup: true, collapsedScale: 1e-7 });
  await page.goto("/");
  const refused = await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("commit_transforms", { moves: [
        { ids: [2], m: [1, 0, 0, 1, 5, 0] },
        { ids: [5], m: [1, 0, 0, 1, 1, 0] },
      ] }).then(() => false, () => true),
  );
  expect(refused).toBe(false);
  expect((await nodeTransform(page, 2))[4]).toBeCloseTo(5, 6);
  // A 1 mm world move beneath a 1e-7 scale is 1e7 in the rect's own space (CodeRabbit on #301).
  expect((await nodeTransform(page, 5))[4] / 1e7).toBeCloseTo(1, 6);
});

test("the fake refuses an empty batch, or a batch with an empty entry, as transform_each does", async ({ page }) => {
  // Copilot on #301: Rust answers EmptySelection for both and keeps nothing; the fake applied the
  // valid first move and recorded the batch.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const send = (moves: unknown[]) => page.evaluate((mv) =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("commit_transforms", { moves: mv }).then(() => false, () => true), moves);
  expect(await send([{ ids: [2], m: [1, 0, 0, 1, 5, 0] }, { ids: [], m: [1, 0, 0, 1, 1, 0] }])).toBe(true);
  expect(await send([])).toBe(true);
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 0, 0]);
  expect(await commitLog(page)).toEqual([]);
});

test("a queued edit is dropped when Reload replaces the document", async ({ page }) => {
  // Copilot on #301: Reload reuses ids, so an X edit queued against the old document moved a shape
  // in the reloaded one.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click(); // gives Reload a path
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // on the wire
  await page.getByLabel("X", { exact: true }).fill("30"); // queued behind it
  await page.getByRole("button", { name: "Reload" }).click();
  await expect(page.getByTestId("layer-row").first()).toBeVisible();
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The queued X must not follow the held drag into the reloaded document. Where the drag itself
  // ends up is not this test's to say: the backend runs it before the load, which then replaces it,
  // while the fake runs a held commit on release, after the load.
  await page.waitForTimeout(300);
  expect(await commitLog(page)).toHaveLength(1);
  expect((await nodeTransform(page, 2))[4]).not.toBeCloseTo(30, 6);
});

test("an edit made while Reload loads is held, and dropped once the document is replaced", async ({ page }) => {
  // An edit made during the load used to go straight out, with ids from the document on screen, and
  // land on whatever the reloaded one gives those ids.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click(); // gives Reload a path
  await page.evaluate(() => (window as unknown as { __holdLoad: () => void }).__holdLoad());
  await page.getByTestId("layer-row").first().click(); // red
  await page.getByRole("button", { name: "Reload" }).click(); // parked
  // Locked while it loads, and saying why: the panel used to take these, and the load then dropped
  // them without a word (Copilot on #301).
  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await expect(centre).toBeDisabled();
  await expect(centre).toHaveAttribute("title", /waiting for the document to load/);
  await expect(page.getByLabel("X", { exact: true })).toBeDisabled();
  await page.evaluate(() => (window as unknown as { __releaseLoad: () => Promise<void> }).__releaseLoad());

  await page.waitForTimeout(300);
  expect(await commitLog(page)).toEqual([]);
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 0, 0]);
  await page.getByTestId("layer-row").first().click();
  await expect(centre).toBeEnabled();
  await expect(page.getByLabel("X", { exact: true })).toBeEnabled();
});

test("a snapshot answering after a newer one has rendered is dropped", async ({ page }) => {
  // CodeRabbit on #301: an older snapshot answering last rendered under a newer revision, so the
  // canvas went back to geometry from before the align that the newer one already showed.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click(); // red
  const x = page.getByLabel("X", { exact: true });
  const before = await x.inputValue();
  await page.evaluate(() => (window as unknown as { __holdSnapshots: () => void }).__holdSnapshots());
  await page.getByRole("checkbox", { name: "Cut this shape" }).click(); // its snapshot is held
  await page.getByRole("button", { name: "Align horizontal centres" }).click(); // to the bed; held too
  await expect.poll(() => commitLog(page).then((l) => l.length)).toBe(1);

  await page.evaluate(() => (window as unknown as { __releaseLatestSnapshot: () => Promise<void> }).__releaseLatestSnapshot());
  await expect(x).not.toHaveValue(before);
  const aligned = await x.inputValue();
  await page.evaluate(() => (window as unknown as { __releaseSnapshot: () => Promise<void> }).__releaseSnapshot());
  await page.waitForTimeout(300);
  await expect(x).toHaveValue(aligned);
});

test("Open or Reload waits for a document edit already on its way before it loads", async ({ page }) => {
  // CodeRabbit on #301: the lock is checked when an edit starts, and an import then reads its file
  // before it sends. A Reload in that gap loaded first, and the import's old root met the new
  // document. A load now waits for edits already started, as it does for a transform commit.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  const loads = () => page.evaluate(() => (window as unknown as { __loads?: number }).__loads ?? 0);
  await page.evaluate(() => (window as unknown as { __holdEdits: () => void }).__holdEdits());
  await page.getByTestId("layer-row").first().click(); // red
  await page.getByRole("checkbox", { name: "Cut this shape" }).click(); // on its way, held
  await page.getByRole("button", { name: "Reload" }).click();
  await page.waitForTimeout(300);
  expect(await loads()).toBe(0);
  await page.evaluate(() => (window as unknown as { __releaseEdits: () => Promise<void> }).__releaseEdits());

  await expect.poll(loads).toBe(1);
  expect(await page.evaluate(() => (window as unknown as { __loadDuringEdit?: boolean }).__loadDuringEdit)).toBeUndefined();
});

test("an import still reading its file finishes before Reload loads", async ({ page }) => {
  // CodeRabbit on #301: the import passes the lock, then awaits its file before it sends. Paused in
  // that read, a Reload must wait for it rather than load and take the import's old root.
  await page.addInitScript(() => {
    const w = window as unknown as {
      __docOrder: string[]; __holdFileReads: () => void; __releaseFileReads: () => void; __heldFileReads: () => number;
    };
    w.__docOrder = [];
    let holding = false;
    const held: (() => void)[] = [];
    const read = Blob.prototype.arrayBuffer;
    Blob.prototype.arrayBuffer = function (this: Blob) {
      if (!holding) return read.call(this);
      return new Promise((resolve, reject) => held.push(() => read.call(this).then(resolve, reject)));
    };
    w.__holdFileReads = () => { holding = true; };
    w.__releaseFileReads = () => { holding = false; held.splice(0).forEach((f) => f()); };
    w.__heldFileReads = () => held.length;
  });
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  await page.evaluate(() => (window as unknown as { __holdFileReads: () => void }).__holdFileReads());
  // Built in the page: the e2e build has no Node types, so no Buffer.
  await page.evaluate(() => {
    const input = document.querySelector<HTMLInputElement>('input[type="file"]')!;
    const files = new DataTransfer();
    files.items.add(new File(['<svg xmlns="http://www.w3.org/2000/svg"><rect width="5" height="5"/></svg>'], "a.svg", { type: "image/svg+xml" }));
    input.files = files.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  // In the read, past the lock check, before Reload is asked for.
  await expect.poll(() => page.evaluate(() => (window as unknown as { __heldFileReads: () => number }).__heldFileReads())).toBe(1);
  await page.getByRole("button", { name: "Reload" }).click();
  await page.waitForTimeout(300);
  const order = () => page.evaluate(() => (window as unknown as { __docOrder: string[] }).__docOrder);
  expect(await order()).toEqual([]);
  await page.evaluate(() => (window as unknown as { __releaseFileReads: () => void }).__releaseFileReads());

  await expect.poll(order).toEqual(["import_svg", "load_project"]);
});

test("a transform's late snapshot retires its preview against the newer one already shown", async ({ page }) => {
  // Copilot on #301: an align's snapshot answering after a Delete's had rendered was dropped as
  // "nothing rendered", so the align's preview stayed up over the newer scene, still drawing (and
  // hit-testing) the shape the Delete had removed.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v = await fittedView(page);
  await page.getByTestId("layer-row").first().click(); // red
  const x = page.getByLabel("X", { exact: true });
  await page.evaluate(() => (window as unknown as { __holdSnapshots: () => void }).__holdSnapshots());
  await page.getByRole("button", { name: "Align horizontal centres" }).click(); // its snapshot is held
  await expect.poll(() => commitLog(page).then((l) => l.length)).toBe(1);
  await expect(x).not.toHaveValue("0");
  const alignedX = Number(await x.inputValue());
  await page.keyboard.press("Delete"); // its snapshot is held too

  await page.evaluate(() => (window as unknown as { __releaseLatestSnapshot: () => Promise<void> }).__releaseLatestSnapshot());
  await expect(page.getByTestId("layer-row")).toHaveCount(1); // the Delete's, red gone
  await page.evaluate(() => (window as unknown as { __releaseSnapshot: () => Promise<void> }).__releaseSnapshot());
  await page.waitForTimeout(300);

  const at = await toPage(page, v, { x: alignedX + 5, y: 5 });
  await page.mouse.click(at.x, at.y);
  await expect(x).toHaveCount(0);
});

test("Delete, Undo and the other document commands are refused while a document loads", async ({ page }) => {
  // CodeRabbit on #301: only transforms waited for a load. A Delete pressed during a Reload named ids
  // from the document on screen, and the loaded one reuses them.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  await page.evaluate(() => (window as unknown as { __holdLoad: () => void }).__holdLoad());
  await page.getByTestId("layer-row").first().click(); // red
  await page.getByRole("button", { name: "Reload" }).click(); // held
  await page.keyboard.press("Delete");
  await expect(page.getByText("the document is still loading")).toBeVisible();
  await page.evaluate(() => (window as unknown as { __releaseLoad: () => Promise<void> }).__releaseLoad());

  await page.waitForTimeout(300);
  await expect(page.getByTestId("layer-row")).toHaveCount(2);
});

test("an edit refused after a load whose read failed says so, reads it again, and unlocks", async ({ page }) => {
  // After Reload loaded but its snapshot failed, every edit was refused as "still loading" until
  // another Reload, which nothing said to do (silent-failure-hunter on #301).
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  const failNextSnapshot = () =>
    page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await failNextSnapshot();
  await page.getByRole("button", { name: "Reload" }).click(); // loads; its read fails
  await expect(page.getByText("snapshot unavailable")).toBeVisible();

  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await page.getByTestId("layer-row").first().click();
  await expect(centre).toBeDisabled();
  await expect(centre).toHaveAttribute("title", /the loaded document could not be read\. Reload to try again/);

  await failNextSnapshot(); // the re-read fails too
  await page.keyboard.press("Delete");
  await expect(page.getByText(/Not applied: the loaded document could not be read: snapshot unavailable\. Reload to try again/)).toBeVisible();
  await expect(centre).toBeDisabled();

  await page.keyboard.press("Delete"); // this re-read works
  await expect(page.getByText(/Not applied, but the loaded document is on screen now/)).toBeVisible();
  await expect(page.getByTestId("layer-row")).toHaveCount(2); // neither Delete went out
  await page.getByTestId("layer-row").first().click();
  await expect(centre).toBeEnabled();
});

test("a snapshot asked for before a later load never renders over it or lifts its lock", async ({ page }) => {
  // Copilot on #301: the lock lifted on any newer snapshot, so a first Reload's snapshot answering
  // after a second Reload had loaded showed the first document and unlocked edits on it, while the
  // backend held the second; an align there sent the first one's bounds into the second.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  const loads = () => page.evaluate(() => (window as unknown as { __loads?: number }).__loads ?? 0);
  await page.evaluate(() => (window as unknown as { __holdSnapshots: () => void }).__holdSnapshots());
  await page.getByRole("button", { name: "Reload" }).click(); // loads; its snapshot is held
  await expect.poll(loads).toBe(1);
  await page.getByRole("button", { name: "Reload" }).click(); // loads again; held too
  await expect.poll(loads).toBe(2);

  const release = () =>
    page.evaluate(() => (window as unknown as { __releaseSnapshot: () => Promise<void> }).__releaseSnapshot());
  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await release(); // the first Reload's answer, read before the second load
  await page.getByTestId("layer-row").first().click();
  await expect(centre).toHaveAttribute("title", /waiting for the document to load/);
  await expect(centre).toBeDisabled();

  await release(); // the second's, read after its load
  await page.getByTestId("layer-row").first().click();
  await expect(centre).toBeEnabled();
});

test("a queued align finds a Group's shapes in the tree as it stands when it is sent", async ({ page }) => {
  // Copilot on #301: the queued click kept the tree from its render, so a shape added to the Group
  // before it drained was left out of the Group's bounds.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedAlignExtras: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click(); // red, 0..10
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await page.getByTestId("layer-row").nth(2).click({ modifiers: ["Shift"] }); // + the Group, 50..80
  await page.getByRole("button", { name: "Align left edges" }).click(); // queued
  // A 10 mm rect joins the Group at 0..10 while the align waits.
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("add_primitive", { parent: 4, kind: { Rect: { w: 10, h: 10 } } }));
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // With the new rect the Group reaches 0, so only red moves (to 0). The old tree said 50..80,
  // which would have sent the Group to 5 instead.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1]).toMatchObject({ ids: [2], m: [1, 0, 0, 1, expect.closeTo(-5, 6), 0] });
});

test("an edit made between a commit settling and the queue draining still waits its turn", async ({ page }) => {
  // CodeRabbit on #301: inFlight cleared on settle but the queue drained a render later, so an
  // edit made in between was sent at once and overlapped the drained one on the wire.
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 5 * v.scale); // red to 5..15 × 5..15
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] }); // + the Group
  await page.getByRole("button", { name: "Align top edges" }).click(); // queued behind the drag

  // Release without yielding a task, let the settled chain run its microtasks, then click: React
  // has not rendered yet, so this lands in the gap before the drain.
  await page.evaluate(async () => {
    void (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits();
    for (let i = 0; i < 50; i++) await Promise.resolve();
    (document.querySelector('[aria-label="Align right edges"]') as HTMLButtonElement).click();
  });

  await expect.poll(async () => (await commitLog(page)).length).toBe(3);
  expect(await page.evaluate(() => (window as unknown as { __maxInFlightCommits?: number }).__maxInFlightCommits)).toBe(1);
  // In the order they were made: top was queued first. One on the wire at a time is not enough on
  // its own; a click in the gap that went out at once would still land before the queued top.
  const [, top, right] = await commitLog(page);
  expect(top).toMatchObject({ ids: [4], m: [1, 0, 0, 1, 0, expect.closeTo(-5, 6)] });
  expect(right).toMatchObject({ ids: [4], m: [1, 0, 0, 1, expect.closeTo(25, 6), 0] });
});

test("Reload waits for a commit already on the wire before it loads", async ({ page }) => {
  // CodeRabbit on #301: gating new edits does not cover a commit that already went out. Open and
  // Reload must not start load_project while one is unanswered, or a reused id could meet it.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // on the wire, held
  await page.getByRole("button", { name: "Reload" }).click();
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The load ran (a never-run Reload would pass a no-overlap check), and did not overlap the commit.
  await expect.poll(() => page.evaluate(() => (window as unknown as { __loads?: number }).__loads ?? 0)).toBe(1);
  expect(await page.evaluate(() => (window as unknown as { __loadDuringCommit?: boolean }).__loadDuringCommit ?? false)).toBe(false);
  // The reload did happen once the commit had settled: the saved copy has red back at 0.
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(0, 6);
});

test("after a load whose snapshot failed, edits on the old view are not sent", async ({ page }) => {
  // Copilot on #301: replacing cleared when the load returned, but the new document's snapshot
  // comes after, so the canvas still showed the old one; an align there sent old bounds and a
  // reused id into the loaded document. Edits stay off until a newer snapshot has rendered.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  await page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await page.getByRole("button", { name: "Reload" }).click();
  await expect(page.getByText("snapshot unavailable")).toBeVisible(); // loaded, but not re-read

  await page.getByTestId("layer-row").first().click(); // red, from the old view
  // Locked, and saying why, rather than taking the click and dropping it (silent-failure-hunter).
  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await expect(centre).toBeDisabled();
  await expect(centre).toHaveAttribute("title", /the loaded document could not be read/);
  await expect(page.getByLabel("X", { exact: true })).toBeDisabled();
  expect(await commitLog(page)).toEqual([]);
});

test("a drag held through a Reload is dropped, not sent to the loaded document", async ({ page }) => {
  // Copilot on #301: an unreleased drag is not a commit yet, so the load did not wait for it, and
  // its pointer-up sent the old ids and matrix straight into the reloaded document.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  const from = await toPage(page, v, { x: 5, y: 5 });
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 5 * v.scale, from.y, { steps: 4 }); // dragging, not released
  await page.getByRole("button", { name: "Reload" }).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("layer-row").first()).toBeVisible();
  await page.mouse.up();

  await page.waitForTimeout(300);
  expect(await commitLog(page)).toEqual([]);
  expect((await nodeTransform(page, 2))[4]).toBeCloseTo(0, 6);
});

test("the fake refuses a held batch whose node was deleted before it was answered", async ({ page }) => {
  // Copilot on #301: the id check ran at the call and the staged apply skipped a node gone by
  // the answer, so the fake applied the rest where Rust refuses the batch.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const refused = await page.evaluate(async () => {
    const w = window as unknown as {
      __holdCommits: () => void; __releaseCommits: () => Promise<void>;
      __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> };
    };
    w.__holdCommits();
    const batch = w.__TAURI_INTERNALS__.invoke("commit_transforms", { moves: [
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
      { ids: [3], m: [1, 0, 0, 1, 5, 0] },
    ] }).then(() => false, () => true);
    await w.__TAURI_INTERNALS__.invoke("delete", { ids: [2] });
    await w.__releaseCommits();
    return batch;
  });
  expect(refused).toBe(true);
  expect(await nodeTransform(page, 3)).toEqual([1, 0, 0, 1, 0, 0]);
});

test("edits work again once a successful Reload's snapshot has rendered", async ({ page }) => {
  // pr-test-analyzer on #301: the lock after a load was tested only while held. Were its release
  // broken, every edit after any Open or Reload would be dead and nothing else would notice.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  await page.getByRole("button", { name: "Reload" }).click();
  await page.getByTestId("layer-row").first().click(); // red
  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await expect(centre).toBeEnabled();
  await centre.click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0]).toMatchObject({ ids: [2], m: [1, 0, 0, 1, expect.closeTo(160, 6), 0] });
});

test("a queued align on two pieces sends nothing if one is deleted before it goes", async ({ page }) => {
  // pr-test-analyzer on #301: an align clicked on two pieces must not turn into "send the survivor
  // to the mat" because the other went while it waited.
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] }); // + the Group
  await page.getByRole("button", { name: "Align left edges" }).click(); // queued on [red, Group]
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("delete", { ids: [2] })); // the Group goes
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The drag lands and its refresh drops the Group; the align, now on red alone, sends nothing.
  await expect.poll(async () => (await nodeTransform(page, 4))[4]).toBeCloseTo(5, 6);
  await page.waitForTimeout(300);
  expect(await commitLog(page)).toHaveLength(1);
});

test("a refused edit puts back the preview of an unread commit beneath it", async ({ page }) => {
  // pr-test-analyzer on #301: every refusal test started with no preview standing. Here the first
  // drag lands but cannot be re-read, so its preview stands for what the backend holds; a refused
  // edit on top must return to that, not to the stale committed scene.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // lands at 5, unread
  await expect(page.getByText("the canvas could not be refreshed")).toBeVisible();
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("5");

  await page.evaluate(() => (window as unknown as { __failNextCommit: () => void }).__failNextCommit());
  await page.getByLabel("X", { exact: true }).fill("30");
  await expect(page.getByText("transform refused")).toBeVisible();
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("5");
});

test("an align pressed from the keyboard during a drag waits for the drag's commit", async ({ page }) => {
  // Copilot and CodeRabbit on #301: busy() ignored a drag under the pointer, so the align went out
  // at once and the release sent a second batch on top of it.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByTestId("layer-row").first().click(); // red
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  const from = await toPage(page, v, { x: 5, y: 5 });
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 5 * v.scale, from.y, { steps: 4 }); // dragging
  await page.getByRole("button", { name: "Align horizontal centres" }).focus();
  await page.keyboard.press("Enter");
  await page.mouse.up();
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The drag first, then the align computed from where it left red; never two on the wire.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  const [drag, align] = await commitLog(page);
  expect(drag.m[4]).toBeCloseTo(5, 6);
  expect(align.m[4]).toBeCloseTo(165 - 10, 6);
  expect(await page.evaluate(() => (window as unknown as { __maxInFlightCommits?: number }).__maxInFlightCommits)).toBe(1);
});

test("a second Open or Reload while one is still loading is refused, and edits stay held", async ({ page }) => {
  // Copilot on #301: two replacements could overlap, and the first to finish cleared the shared
  // lock while the other was still loading, letting old-document edits reach it.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Save" }).click();
  await page.evaluate(() => (window as unknown as { __holdLoad: () => void }).__holdLoad());
  await page.getByRole("button", { name: "Reload" }).click(); // held
  await page.getByRole("button", { name: "Reload" }).click(); // refused while the first loads
  await expect(page.getByText("another document is still loading")).toBeVisible();
  await page.evaluate(() => (window as unknown as { __releaseLoad: () => Promise<void> }).__releaseLoad());
  await expect.poll(() => page.evaluate(() => (window as unknown as { __loads?: number }).__loads ?? 0)).toBe(1);

  // Once the first load's snapshot renders, editing works.
  await page.getByTestId("layer-row").first().click();
  await expect(page.getByRole("button", { name: "Align horizontal centres" })).toBeEnabled();
});
