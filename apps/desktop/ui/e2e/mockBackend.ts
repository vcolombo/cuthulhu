// SPDX-License-Identifier: GPL-3.0-or-later
// Minimal in-memory fake Tauri backend. Runs inside the page (via addInitScript, so it
// can't close over anything outside itself) and mirrors the JSON shape produced by
// crates/document's Document::snapshot_json() — see App.tsx's DocSnapshot/buildScene,
// which is what actually parses this on the JS side.
export function installMockTauri(opts?: { seedTwoColorRects?: boolean; seedRedGold?: boolean; seedThreeVinyl?: boolean; seedVinylBoard?: boolean; seedFaint?: boolean; failImagePreview?: boolean; dropTraceControl?: string; seedBusyHost?: boolean; seedRemoteConnected?: boolean; seedRemoteIdle?: boolean; seedTwoCutters?: boolean; slowList?: boolean; failList?: boolean; noFonts?: boolean; seedMachine?: boolean; seedUserPreset?: boolean; seedEmptyPresetAssignment?: boolean; seedGroup?: boolean; seedAlignExtras?: boolean; seedCollapsedGroup?: boolean; collapsedScale?: number; holdPreferences?: boolean; unsetUnits?: boolean; holdHistory?: boolean; holdFirstUnitSave?: boolean; failSetMachine?: boolean; seedProtocolMismatch?: boolean; demo?: { id: string } }) {
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
  // Set once the fixture is applied, below the device lists. import_svg reads it at call time.
  let demoImportError: string | null = null;

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

  const addTwoColorRects = () => {
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
  };
  if (opts?.seedTwoColorRects) addTwoColorRects();

  // Screenshot seed only. Green is not a named swatch; gold is, so the swap bar can say "Gold vinyl".
  // A rect is drawn from its transform (`buildScene` uses w/h at the local origin). An x on the
  // rect leaves both squares on the same point, and red over gold reads as one orange square.
  if (opts?.seedRedGold) {
    const redId = nextId++;
    doc.nodes[redId] = {
      id: redId,
      kind: { Shape: { Rect: { w: 10, h: 10 } } },
      transform: [1, 0, 0, 1, 0, 0],
      style: { stroke: 0xff0000ff, fill: null },
      children: [],
      cut_line_type: "Cut",
      material_preset: { state: "inherit" },
    };
    const goldId = nextId++;
    doc.nodes[goldId] = {
      id: goldId,
      kind: { Shape: { Rect: { w: 10, h: 10 } } },
      transform: [1, 0, 0, 1, 30, 0],
      style: { stroke: 0xd39900ff, fill: null },
      children: [],
      cut_line_type: "Cut",
      material_preset: { state: "inherit" },
    };
    doc.nodes[doc.root].children.push(redId, goldId);
  }

  // Red, then a green that tests switch off, then gold. The swap must name gold.
  if (opts?.seedThreeVinyl) {
    const add = (x: number, stroke: number) => {
      const id = nextId++;
      doc.nodes[id] = {
        id,
        kind: { Shape: { Rect: { x, y: 0, w: 10, h: 10 } } },
        transform: [1, 0, 0, 1, 0, 0],
        style: { stroke, fill: null },
        children: [],
        cut_line_type: "Cut",
        material_preset: { state: "inherit" },
      };
      doc.nodes[doc.root].children.push(id);
    };
    add(0, 0xff0000ff);
    add(20, 0x00ff00ff);
    add(40, 0xd39900ff);
  }

  // Seven vinyl colours on a Cameo 12×12. Gold is before red so the red fill covers the gold fill
  // and the outline pass has to put the gold edge back. Centres are in the test that reads pixels.
  if (opts?.seedVinylBoard) {
    doc.machine = machines[0];
    const pack = (hex: string): number => ((Number.parseInt(hex.slice(1), 16) << 8) | 0xff) >>> 0;
    const ellipse = (rx: number, tx: number, ty: number, hex: string): Node => {
      const id = nextId++;
      return { id, kind: { Shape: { Ellipse: { rx, ry: rx } } }, transform: [1, 0, 0, 1, tx, ty], style: { stroke: pack(hex), fill: null }, children: [], cut_line_type: "Cut", material_preset: { state: "inherit" } };
    };
    const rect = (x: number, y: number, hex: string): Node => {
      const id = nextId++;
      return { id, kind: { Shape: { Rect: { w: 30.48, h: 30.48 } } }, transform: [1, 0, 0, 1, x, y], style: { stroke: pack(hex), fill: null }, children: [], cut_line_type: "Cut", material_preset: { state: "inherit" } };
    };
    const shapes = [
      ellipse(22.86, 27.94, 27.94, "#d39900"),
      ellipse(50.8, 0, 0, "#d02c2a"),
      rect(115, 10, "#1c1c1c"),
      rect(160, 10, "#ffffff"),
      rect(205, 10, "#113663"),
      rect(115, 55, "#7b3fa0"),
      rect(160, 55, "#f5e6a3"),
    ];
    for (const shape of shapes) doc.nodes[shape.id] = shape;
    doc.nodes[doc.root].children.push(...shapes.map((shape) => shape.id));
  }

  // Alpha byte 1 is under 0.1. The planner still keys it (alpha 0 is the absent colour), and
  // the properties note is the only place that says the shape still cuts.
  if (opts?.seedFaint) {
    const id = nextId++;
    doc.nodes[id] = {
      id,
      kind: { Shape: { Rect: { w: 10, h: 10 } } },
      transform: [1, 0, 0, 1, 0, 0],
      style: { stroke: 0xff000001, fill: null },
      children: [],
      cut_line_type: "Cut",
      material_preset: { state: "inherit" },
    };
    doc.nodes[doc.root].children.push(id);
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
    // The stacks are not the document. Nothing in the suite undoes, and the buttons stay disabled
    // while both flags are false.
    history: () => {
      if (!opts?.holdHistory) return { can_undo: false, can_redo: false };
      // Queued so a test can answer a later refresh before an earlier one. The flags belong to
      // the snapshot that asked, and a late reply must not rewind Undo.
      return new Promise((resolve) => {
        const w = window as unknown as {
          __historyWaiters?: Array<(flags: { can_undo: boolean; can_redo: boolean }) => void>;
          __releaseLastHistory?: (flags: { can_undo: boolean; can_redo: boolean }) => void;
        };
        w.__historyWaiters = w.__historyWaiters ?? [];
        w.__historyWaiters.push(resolve);
        w.__releaseLastHistory = (flags) => {
          const waiter = w.__historyWaiters?.pop();
          waiter?.(flags);
        };
      });
    },
    undo: () => unimplemented("undo"),
    redo: () => unimplemented("redo"),
    boolean_op: (a) => {
      (window as unknown as { __booleanOp?: unknown }).__booleanOp = a.op;
      return unimplemented("boolean_op");
    },
    import_svg: (a) => {
      (window as unknown as { __docOrder?: string[] }).__docOrder?.push("import_svg");
      if (demoImportError !== null) throw ipcError("import_error", demoImportError);
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
      if (opts?.failSetMachine) throw new Error("could not set the machine");
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
    jobId: number;
  };
  type DeviceEvent = { job_id: number; kind: unknown; status: CutStatus };

  const NO_ACTIONS: Actions = { cut: false, cancel: false, resume: false, confirm: false };
  // Read by `statusOf` as each status is built, including the constants below.
  let jobId: number | null = null;
  const statusOf = (phase: string, actions: Partial<Actions> = {}, rest: Partial<CutStatus> = {}): CutStatus => ({
    phase,
    ended: null,
    actions: { ...NO_ACTIONS, ...actions },
    pass: null,
    sent: null,
    error: null,
    jobId: jobId ?? 0,
    ...rest,
  });
  const DISCONNECTED = statusOf("Disconnected");
  const CONNECTING = statusOf("Connecting");
  // `Idle` alone is a device that has cut nothing; a job that ran to the end rests on
  // the same phase and says so through `ended`, which is the whole of the difference.
  const IDLE = statusOf("Idle", { cut: true });

  const devices: DeviceInfo[] = [
    { instance_id: "usb:mock", machine_id: "cameo5", transport: { Usb: { locator: "mock" } }, candidate: false, host: null },
    { instance_id: "serial:/dev/mock0", machine_id: "puma", transport: { Serial: { path: "/dev/mock0", baud: 9600 } }, candidate: true, host: null },
  ];

  // A paired host that cannot be reached, with a cutter on it. Both halves matter: the row has
  // to stay listed with its reason (#42), and `forget_host` has to be refusable while it does.
  let hosts: { id: string; name: string; address: string; unreachable: string | null; protocol_side: "host" | "desktop" | null; last_seen?: number | null }[] = [];
  const addBusyHost = () => {
    hosts = [{ id: "host-1", name: "Workshop Pi", address: "pi.local:7878",
               unreachable: "the host could not be reached (timed out)", protocol_side: null }];
    devices.push({ instance_id: "usb:pi:A", machine_id: "cameo5",
                   transport: { Usb: { locator: "pi" } }, candidate: false, host: "host-1" });
  };
  if (opts?.seedBusyHost) addBusyHost();

  // The sentence is what the row prints. Which side is older is `protocol_side`, the token
  // the chip and the link switch on. A sentence alone leaves the chip on Unknown.
  if (opts?.seedProtocolMismatch) {
    hosts.push(
      { id: "host-old", name: "Workshop Pi", address: "pi.local:7878",
        unreachable: "This Cut Host is running an older version. Update your Cut Host to connect.",
        protocol_side: "host" },
      { id: "host-new", name: "Office Pi", address: "office.local:7878",
        unreachable: "This version of Cuthulhu is older than the Cut Host. Update Cuthulhu to connect.",
        protocol_side: "desktop" },
    );
    devices.push(
      { instance_id: "usb:pi:old", machine_id: "cameo5",
        transport: { Usb: { locator: "pi" } }, candidate: false, host: "host-old" },
      { instance_id: "usb:pi:new", machine_id: "cameo5",
        transport: { Usb: { locator: "pi" } }, candidate: false, host: "host-new" },
    );
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
  if (opts?.seedRemoteConnected || opts?.seedRemoteIdle) {
    hosts.push({ id: "host-3", name: "Bench Pi", address: "bench.local:7878", unreachable: null, protocol_side: null, last_seen: null });
    devices.push(REMOTE_CUTTER);
  }

  // Two cutters on one reachable host. Statuses live in `remoteStatus` so a cancel for one
  // does not move the other, which is the desktop's (host, cutter) key rather than the aim.
  const remoteStatus = new Map<string, CutStatus>();
  const cancelTargets: { host: string | null; instanceId: string }[] = [];
  const statusKey = (host: string | null, instance: string) => `${host ?? ""}\0${instance}`;
  if (opts?.seedTwoCutters) {
    hosts.push({
      id: "host-4", name: "Garage Pi", address: "garage.local:7878",
      unreachable: null, protocol_side: null, last_seen: Date.now() - 2 * 60 * 60 * 1000,
    });
    devices.push(
      { instance_id: "usb:garage:cameo", machine_id: "cameo5", transport: { Usb: { locator: "garage" } }, candidate: false, host: "host-4" },
      { instance_id: "usb:garage:puma", machine_id: "puma", transport: { Usb: { locator: "garage-puma" } }, candidate: false, host: "host-4" },
    );
    remoteStatus.set(statusKey("host-4", "usb:garage:cameo"), statusOf(
      "Sending", { cancel: true }, { sent: { sent: 40, total: 100 }, pass: { index: 0, total: 1 } },
    ));
    remoteStatus.set(statusKey("host-4", "usb:garage:puma"), statusOf("Idle", { cut: true }));
  }

  let connected: DeviceInfo | null = opts?.seedRemoteConnected ? REMOTE_CUTTER : null;
  // A cancel whose stop nothing confirmed: the Job is over, and `driver-core` still refuses a cut
  // until the transport is re-opened. `Idle` on both sides of the reconnect, which is exactly why
  // the control is derived from `actions` and not from the phase.
  let status: CutStatus = opts?.seedRemoteConnected
    ? { phase: "Idle", ended: "Cancelled", actions: { ...NO_ACTIONS }, pass: null, sent: null, error: null, jobId: 0 }
    : DISCONNECTED;
  let deviceStateCalls = 0;
  let listDeviceCalls = 0;
  let listReads = 0;
  let nextJobId = 1;
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
  // See `__holdPass` below: the advance out of Sending, parked until a test releases it.
  let holdPass = false;
  let heldPass: (() => void) | null = null;
  // The cut command answers before any Sending event, and `get_device_state` still has the
  // previous ending. That is the window a second cut used to keep Completed on screen.
  let holdCutEvents = false;
  let heldCutStart: (() => void) | null = null;
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
    // The scripted pass leaves Sending after 50ms. Step 1 is where the badge is, and getting
    // there takes longer than that, so a test that wants the transmitting tone holds the advance.
    __holdPass: () => { holdPass = true; },
    __releasePass: () => {
      holdPass = false;
      const advance = heldPass;
      heldPass = null;
      advance?.();
    },
    __holdCutEvents: () => { holdCutEvents = true; },
    __releaseCutEvents: () => {
      holdCutEvents = false;
      const start = heldCutStart;
      heldCutStart = null;
      start?.();
    },
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
  // `enabledPos` is the index among enabled passes, which is what driver-core reports
  // (`PassPosition` into the submitted vec). The plan-row index would name a disabled
  // colour when one sits in front of the pass the machine is actually on.
  function runPass(enabledPos: number, enabledCount: number) {
    const total = 100;
    const sending = (sent: number) =>
      statusOf("Sending", { cancel: true }, { pass: { index: enabledPos, total: enabledCount }, sent: { sent, total } });
    status = sending(0);
    emit("StateChanged");
    const advance = () => {
      status = sending(total);
      emit({ Progress: { pass_index: enabledPos, submitted_bytes: total, total_bytes: total } });
      emit({ PassComplete: enabledPos });

      const isLast = enabledPos === enabledCount - 1;
      if (isLast) {
        // finish_pass emits JobComplete *before* the state becomes Idle, so this event's
        // status still reads Sending. A frontend that treats the JobComplete kind as
        // "finished" would show a completed job as still cutting; only the Idle status
        // below says the job is over.
        emit("JobComplete");
        status = statusOf("Idle", { cut: true }, { ended: "Completed" });
        emit("StateChanged");
        // Production releases the job id once the job is over: all later lifecycle
        // events (reconnects, state refreshes) carry NO_JOB=0. Mirror that, or the
        // mock keeps stamping finished-job ids on lifecycle events production
        // would never stamp.
        jobId = null;
      } else {
        status = statusOf(
          "AwaitingColorSwap",
          { cancel: true, resume: true },
          { pass: { index: enabledPos + 1, total: enabledCount } },
        );
        emit("StateChanged");
      }
    };
    // Held before the timer: Sending is only on screen for 50ms, and the badge that has to
    // show it lives on step 1, a navigation away.
    if (holdPass) heldPass = advance;
    else setTimeout(advance, 50);
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
      listReads++;
      const slow = opts?.slowList && listDeviceCalls++ > 0;
      // A reachable host just answered. An unreachable one keeps the stamp from the last
      // reading that worked, which is what "Last seen" is counting.
      const publish = () => {
        const now = Date.now();
        for (const host of hosts) {
          if (host.unreachable === null) host.last_seen = now;
        }
        return [...devices];
      };
      return slow ? new Promise((r) => setTimeout(() => r(publish()), 3000)) : publish();
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
      if (info.host !== null) {
        // Aiming at a host releases the local manager. That Disconnected is emitted on the
        // bridge after this command has answered, so it can land after the UI stores the host.
        // The cell stays the host's status: get_device_state is aimed at the host, and the
        // event is only the local transport letting go.
        setTimeout(() => {
          const ev: DeviceEvent = { job_id: jobId ?? 0, kind: "StateChanged", status: DISCONNECTED };
          for (const id of eventNameToIds.get("device-event") ?? []) {
            callbacksById.get(id)?.({ event: "device-event", id, payload: ev });
          }
          (window as unknown as { __hostReleaseEmitted?: boolean }).__hostReleaseEmitted = true;
        }, 0);
      }
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
      // Counted because a desktop with no host must not dial a status. The device-list poll
      // is counted separately: that is the one interval, and it is not this command.
      deviceStateCalls++;
      return status;
    },
    // The cache `list_devices` would have stored. Explicit rows win; the aimed remote cutter
    // otherwise follows the global status the job events already drive.
    cutter_statuses: () => {
      const rows: { host: string; instance_id: string; status: CutStatus }[] = [];
      for (const [key, rowStatus] of remoteStatus) {
        const split = key.indexOf("\0");
        const host = key.slice(0, split);
        if (host === "") continue;
        rows.push({ host, instance_id: key.slice(split + 1), status: rowStatus });
      }
      if (connected?.host) {
        const key = statusKey(connected.host, connected.instance_id);
        if (!remoteStatus.has(key)) {
          rows.push({ host: connected.host, instance_id: connected.instance_id, status });
        }
      }
      return rows;
    },
    get_connected_device: () => connected,
    plan_cut: (a) => {
      const planned = (window as unknown as { __planCalls?: number }).__planCalls ?? 0;
      (window as unknown as { __planCalls?: number }).__planCalls = planned + 1;
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
        status = statusOf("Sending", { cancel: true }, { pass: { index: 0, total: enabledIndices.length }, sent: { sent: 0, total: 100 } });
        emit("StateChanged");
        emit({ Failed: "Timeout" });
        status = statusOf("Failed", {}, { error: "Timeout" });
        emit("StateChanged");
        jobId = null;
        return { job_id: id, duplicate: false };
      }
      const begin = () => runPass(0, enabledIndices.length);
      if (holdCutEvents) heldCutStart = begin;
      else begin();
      // `duplicate` is the Cut Host's own answer — it is the only party that knows whether it had
      // already accepted this dispatch id. This fake stands in for a local cutter, which has no
      // dedupe to be caught by, so it is always false here.
      return { job_id: jobId, duplicate: false };
    },
    cancel_cut: (a) => {
      const targeted = typeof a.instanceId === "string"
        ? { host: a.host == null ? null : String(a.host), instanceId: a.instanceId }
        : null;
      if (targeted) cancelTargets.push(targeted);
      const matchesAim = targeted === null || (connected !== null
        && connected.instance_id === targeted.instanceId
        && (connected.host ?? null) === targeted.host);
      // A cutter that is not the aim still has a status of its own. Cancelling it must not
      // move the aimed cutter, and must not be dropped because the aim is elsewhere.
      if (!matchesAim && targeted) {
        const key = statusKey(targeted.host, targeted.instanceId);
        const prev = remoteStatus.get(key);
        if (prev) {
          remoteStatus.set(key, statusOf("Idle", { cut: true }, { ended: "Cancelled", pass: prev.pass, sent: prev.sent }));
        }
        return null;
      }
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
      const enabledCount = planPasses.filter((p) => p.enabled).length;
      runPass(nextIndex, enabledCount);
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
    // Test hook (no production counterpart): fires an event the *backend* emits, which no command
    // of the fake's does. `cut-in-progress` is emitted from `on_window_event` when the close guard
    // refuses a close, and a window Playwright asks to close never reaches that guard here.
    __test_emit_backend_event: (a) => {
      const event = a.event as string;
      for (const id of eventNameToIds.get(event) ?? []) {
        callbacksById.get(id)?.({ event, id, payload: null });
      }
      return null;
    },
    // Same, for whether the page has got as far as subscribing: an emit before the listener is
    // registered reaches nobody, and `goto` resolving is not that moment.
    __test_backend_listeners: (a) => (eventNameToIds.get(a.event as string) ?? []).length,
    // Readiness is recorded with the listener count at the moment it was set, because the ordering
    // is the whole point of the flag: Rust emits on the strength of it, and `emit` succeeds with
    // nobody listening — so readiness while no listener exists is a prevented close with no prompt
    // and no escape. The latest value stays where `emitBackendEvent` reads it.
    set_close_guard_ready: (a) => {
      sessionStorage.setItem("__close_guard_ready__", String(a.ready));
      const listeners = (eventNameToIds.get("cut-in-progress") ?? []).length;
      const log = sessionStorage.getItem("__ready_log__") ?? "";
      sessionStorage.setItem("__ready_log__", `${log}${log ? "," : ""}${a.ready}@${listeners}`);
      return null;
    },
    // Mirrors `main.rs`'s `force_quit` as far as a page can: the real one exits the process hosting
    // this webview, which a fake cannot do, so all a test can observe is that it was asked.
    // Recorded rather than counted, because the question is whether confirming reached it at all.
    force_quit: () => {
      sessionStorage.setItem("__force_quit__", "asked");
      return null;
    },
    // Mirrors `main.rs`'s `cancel_quit`: the real one replies to the OS only when a macOS terminate
    // is owed, which a page cannot see, so what a test can observe is that the dismissal reached it.
    cancel_quit: () => {
      sessionStorage.setItem("__quit_cancelled__", "asked");
      return null;
    },
    // Test hook (no production counterpart): how many times the dialog has asked for a status.
    __test_poll_count: () => deviceStateCalls,
    __test_list_count: () => listReads,
    __test_cancel_targets: () => cancelTargets,
    confirm_pass_done: () => {
      status = statusOf("Idle", { cut: true }, { ended: "Completed" });
      emit("StateChanged");
      return null;
    },
    // Test hooks (no production counterpart): drive the job panel without waiting out a scripted pass.
    __test_set_progress: (a) => {
      const sent = Number(a.sent);
      const total = status.sent?.total ?? Number(a.total ?? 100);
      status = { ...status, sent: { sent, total } };
      emit("StateChanged");
      return null;
    },
    __test_set_remote_sent: (a) => {
      const key = statusKey(a.host == null ? null : String(a.host), String(a.instanceId));
      const prev = remoteStatus.get(key);
      if (!prev) return null;
      const total = prev.sent?.total ?? Number(a.total ?? 100);
      remoteStatus.set(key, { ...prev, sent: { sent: Number(a.sent), total } });
      return null;
    },
    __test_set_last_seen: (a) => {
      const host = hosts.find((item) => item.id === String(a.id));
      if (host) host.last_seen = Number(a.at);
      return null;
    },
    __test_set_status: (a) => {
      status = statusOf(String(a.phase), (a.actions ?? {}) as Partial<Actions>, (a.rest ?? {}) as Partial<CutStatus>);
      emit("StateChanged");
      return null;
    },
    __test_host_unreachable: (a) => {
      const id = String(a.id);
      hosts = hosts.map((h) => (h.id === id ? { ...h, unreachable: a.unreachable == null ? null : String(a.unreachable) } : h));
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
    list_hosts: () => {
      const hooks = window as unknown as { __hostListCalls?: number };
      hooks.__hostListCalls = (hooks.__hostListCalls ?? 0) + 1;
      return hosts;
    },
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
      const host = { id: "host-2", name: a.name as string, address: a.address as string, unreachable: null, protocol_side: null, last_seen: null };
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
    // Persisted in sessionStorage so a reload keeps it: addInitScript rebuilds this closure,
    // and localStorage is the boot cache of the resolved theme, not the preference.
    get_preferences: () => {
      const read = () => {
        const raw = sessionStorage.getItem("cuthulhu.preferences");
        // A saved millimetre, not the locale default. The suite's cursor readout is millimetres.
        // `unsetUnits` is the first-run file: null until something saves a unit.
        if (!raw) return { appearance: "system", units: opts?.unsetUnits ? null : "mm" };
        const parsed = JSON.parse(raw) as { appearance?: string; units?: string };
        const saved = parsed.units === "in" || parsed.units === "mm" ? parsed.units : null;
        const units = saved ?? (opts?.unsetUnits ? null : "mm");
        return { appearance: parsed.appearance ?? "system", units };
      };
      if (opts?.holdPreferences) {
        // Theme and units both read preferences on mount. One resolver would release only the
        // later call, and the theme would stay on the boot script's class.
        return new Promise((resolve) => {
          const w = window as unknown as { __prefWaiters?: Array<() => void>; __releasePreferences?: () => void };
          w.__prefWaiters = w.__prefWaiters ?? [];
          w.__prefWaiters.push(() => resolve(read()));
          w.__releasePreferences = () => {
            const waiters = w.__prefWaiters ?? [];
            w.__prefWaiters = [];
            for (const release of waiters) release();
          };
        });
      }
      return read();
    },
    set_preferences: (a) => {
      const write = () => {
        const raw = sessionStorage.getItem("cuthulhu.preferences");
        const parsed = raw ? (JSON.parse(raw) as { appearance?: string; units?: string }) : {};
        const appearance = a.appearance == null ? (parsed.appearance ?? "system") : a.appearance;
        const units = a.units == null ? (parsed.units === "in" || parsed.units === "mm" ? parsed.units : "mm") : a.units;
        if (appearance !== "light" && appearance !== "dark" && appearance !== "system") {
          throw new Error("appearance is not light, dark, or system");
        }
        if (units !== "in" && units !== "mm") throw new Error("units are not in or mm");
        const prefs = { appearance, units };
        sessionStorage.setItem("cuthulhu.preferences", JSON.stringify(prefs));
        return prefs;
      };
      // The first unit write is the region default. Held, a click can land while it is in flight.
      if (opts?.holdFirstUnitSave && a.units != null && !(window as unknown as { __heldUnitSave?: boolean }).__heldUnitSave) {
        (window as unknown as { __heldUnitSave?: boolean }).__heldUnitSave = true;
        return new Promise((resolve) => {
          (window as unknown as { __releaseUnitSave?: () => void }).__releaseUnitSave = () => resolve(write());
        });
      }
      return write();
    },
  } as Record<string, (args: Record<string, unknown>) => unknown>);

  // The dev menu stores only the id. Session storage wins so a menu reload overrides the
  // init-script argument. The key and the import sentence are spelled out: this function is
  // serialized into the page and cannot import src/demo/fixtures.ts. Switching on the id keeps
  // a stored "hostOffline" from applying a different screen than the menu.
  const storedDemo = sessionStorage.getItem("cuthulhu.demo");
  let demoId: string | null = null;
  if (storedDemo) {
    try {
      const parsed: unknown = JSON.parse(storedDemo);
      if (parsed && typeof parsed === "object" && "id" in parsed) {
        const id = (parsed as { id: unknown }).id;
        if (typeof id === "string") demoId = id;
      }
    } catch {
      demoId = null;
    }
  } else if (typeof opts?.demo?.id === "string") {
    demoId = opts.demo.id;
  }
  const seedShapes = () => {
    if (doc.nodes[doc.root].children.length === 0) addTwoColorRects();
  };
  if (demoId !== null) {
    switch (demoId) {
      case "normal":
        seedShapes();
        break;
      case "noCutters":
        devices.splice(0, devices.length);
        hosts = [];
        connected = null;
        seedShapes();
        break;
      case "emptyCanvas":
        for (const id of doc.nodes[doc.root].children) delete doc.nodes[id];
        doc.nodes[doc.root].children = [];
        break;
      case "importFailed":
        seedShapes();
        demoImportError = "Couldn't import “logo-final.svg”. The file is not valid SVG.";
        break;
      case "hostOffline":
        seedShapes();
        if (!hosts.some((host) => host.id === "host-1")) addBusyHost();
        // Connected here, not via aim(): a confirm dialog would sit on the first cutter the menu shows.
        // Sending, so the job panel stays up when the first device read lands before the menu
        // remembers the job. A Disconnected read there would leave nothing watching the host.
        connected = devices.find((device) => device.instance_id === "usb:pi:A") ?? null;
        status = statusOf("Sending", { cancel: true });
        break;
      default:
        demoId = null;
        break;
    }
    if (demoId !== null) {
      (window as unknown as { __cuthulhuDemo?: unknown }).__cuthulhuDemo = { id: demoId };
    }
  }

  (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    invoke: (cmd: string, args: Record<string, unknown> = {}) => {
      if (cmd === "plugin:dialog|save" || cmd === "plugin:dialog|open") {
        return Promise.resolve("/mock/cuthulhu-project.cut");
      }
      // The dialog plugin's `confirm` is a `message` command, so this is the quit prompt. Answered
      // late, as the real one is, and recorded: a test cannot tell a prompt that was asked from one
      // that never was by what the page did next. The answer is the test's, defaulting to Cancel,
      // and is the label of the button the page offered, which is what the plugin compares.
      if (cmd === "plugin:dialog|message") {
        const log = JSON.parse(sessionStorage.getItem("__dialog_messages__") ?? "[]") as string[];
        log.push(String(args.message));
        sessionStorage.setItem("__dialog_messages__", JSON.stringify(log));
        // The plugin's wire form for custom labels (`buttonsToRust`), not something to validate.
        const offered = args.buttons as { OkCancelCustom: [string, string] };
        const [okLabel, cancelLabel] = offered.OkCancelCustom;
        const ok = sessionStorage.getItem("__dialog_answer__") === "ok";
        return new Promise((resolve) => setTimeout(() => resolve(ok ? okLabel : cancelLabel), 250));
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
      // Installed by the test's beforeEach, so nothing in the page's own types knows about it.
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
