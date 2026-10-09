// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useMemo, useRef, useState, type SetStateAction } from "react";
import { listen } from "@tauri-apps/api/event";
import * as ipc from "./ipc";
import { Canvas2DRenderer } from "./render/Canvas2DRenderer";
import { unionBounds, type Affine6, type Scene, type ShapeGeom } from "./render/hittest";
import { pathBounds } from "./render/pathdata";
import { IDENTITY, compose, transformBounds } from "./render/affine";
import { outermost, shapesUnder, toggleId } from "./interaction/marquee";
import { AXIS, alignMoves, distributeBlock, distributeMoves, type AlignMode, type Axis, type Unit } from "./interaction/align";
import type { Matrix } from "./interaction/transform";
import { useCanvasInteraction, type CommitOutcome, type EditsLock } from "./interaction/useCanvasInteraction";
import { viewMatrix, zoomPercent } from "./interaction/viewport";
import { TopBar } from "./panels/TopBar";
import { ToolRail } from "./panels/ToolRail";
import { LayersPanel } from "./panels/LayersPanel";
import { PropertiesPanel } from "./panels/PropertiesPanel";
import { selectionCutLineType, type CutLineTypeJson } from "./panels/cutLineType";
import { effectiveMaterials, selectionAssignment, summariseEffectiveMaterial } from "./panels/materialPreset";
import type { Preset } from "./cut/viewmodel";
import { StatusBar } from "./panels/StatusBar";
import { CutDialog } from "./cut/CutDialog";
import type { WeedDraft } from "./cut/viewmodel";
import { TraceDialog } from "./trace/TraceDialog";
import { TextDialog } from "./text/TextDialog";

// Shapes mirroring the Rust `document` crate's serde JSON. Loose but sufficient for the
// paths this UI actually reads — see crates/document/src/{node,delta,machine}.rs.
export type BoolOp = "Union" | "Subtract" | "Intersect" | "Exclude";
export type { Affine6 };

export type ShapeKindJson =
  | { Rect: { w: number; h: number } }
  | { Ellipse: { rx: number; ry: number } }
  | { Text: { family: string; size_mm: number; text: string } }
  | { Path: { d: string } };

export type NodeKindJson = "Layer" | "Group" | { Shape: ShapeKindJson };

// Delta shape returned by commands like boolean_op — see crates/document/src/delta.rs's
// NodeOp. Only the Add variant's node id is read (to select a command's result node).
type NodeOpJson =
  | { Add: { parent: number; node: { id: number }; index: number } }
  | { Remove: { parent: number; id: number } }
  | { Update: { id: number; before: unknown; after: unknown } };

export type DocNode = {
  id: number;
  kind: NodeKindJson;
  transform: Affine6;
  cut_line_type: CutLineTypeJson;
  material_preset: ipc.PresetAssignmentJson;
  children: number[];
};

export type MachineProfile = { id: string; name: string; width_mm: number; height_mm: number };

export type DocSnapshot = {
  nodes: Record<string, DocNode>;
  root: number;
  artboard: { x: number; y: number; w: number; h: number };
  machine: MachineProfile | null;
};

function shapeBounds(kind: ShapeKindJson) {
  if ("Rect" in kind) return { x: 0, y: 0, w: kind.Rect.w, h: kind.Rect.h };
  if ("Ellipse" in kind) {
    // Canonical convention (see crates/document/src/commands.rs local_shape_path): an
    // Ellipse's local space is centered at (rx, ry), bounds 0..2rx / 0..2ry.
    return { x: 0, y: 0, w: kind.Ellipse.rx * 2, h: kind.Ellipse.ry * 2 };
  }
  // Text nodes are converted server-side into a Path before insertion (add_text mints a
  // Path node), so this is the real Path bounds path too.
  if ("Path" in kind) return pathBounds(kind.Path.d) ?? { x: 0, y: 0, w: 0, h: 0 };
  return { x: 0, y: 0, w: 0, h: 0 }; // bare Text kind shouldn't reach here at runtime
}

function shapeGeom(kind: ShapeKindJson): ShapeGeom | undefined {
  if ("Rect" in kind) return { t: "rect", w: kind.Rect.w, h: kind.Rect.h };
  if ("Ellipse" in kind) return { t: "ellipse", rx: kind.Ellipse.rx, ry: kind.Ellipse.ry };
  if ("Path" in kind) return { t: "path", d: kind.Path.d };
  return undefined;
}

function buildScene(doc: DocSnapshot): Scene {
  const nodes: Scene["nodes"] = [];
  const walk = (id: number, parentWorld: Affine6) => {
    const n = doc.nodes[id];
    if (!n) return;
    const world = compose(n.transform, parentWorld);
    if (typeof n.kind === "object" && "Shape" in n.kind) {
      // `local` travels with the node so hit-testing and handles work in its own frame;
      // `bounds` is its axis-aligned world box (all four corners, so committed scale and
      // rotation both show up), for the marquee and the properties panel.
      const local = shapeBounds(n.kind.Shape);
      nodes.push({ id: n.id, bounds: transformBounds(world, local), local, shape: shapeGeom(n.kind.Shape), world });
    } else {
      for (const child of n.children) walk(child, world);
    }
  };
  walk(doc.root, IDENTITY);
  return { nodes };
}

/** One align unit per id, bounded by every shape it moves (`expand`) as they stand in `scene`, in
 *  document order (the scene's, by each unit's first shape), since that breaks a distribute tie. */
function unitsOf(s: Scene, ids: number[], expand: (ids: number[]) => number[]): Unit[] {
  if (ids.length === 0) return [];
  const at = new Map(s.nodes.map((n, i) => [n.id, i]));
  const found = ids.flatMap((id) => {
    const idx = expand([id]).flatMap((sid) => {
      const i = at.get(sid);
      return i === undefined ? [] : [i];
    });
    if (idx.length === 0) return [];
    const first = idx.reduce((a, b) => Math.min(a, b));
    return [{ first, unit: { ids: [id], bounds: unionBounds(idx.map((i) => s.nodes[i].bounds)) } }];
  });
  return found.sort((a, b) => a.first - b.first).map((f) => f.unit);
}

export function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<Canvas2DRenderer | null>(null);

  const [doc, setDoc] = useState<DocSnapshot | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const [machines, setMachines] = useState<MachineProfile[]>([]);
  const [tool, setTool] = useState("select");
  const [error, setErrorState] = useState<string | null>(null);
  // When the message on screen was raised, on the clock the canvas stamps its requests with: a
  // commit clears only a message older than the request it carries (Copilot on #301).
  const errorAt = useRef(0);
  const setError = useCallback((next: SetStateAction<string | null>) => {
    if (typeof next === "string") errorAt.current = performance.now();
    setErrorState(next);
  }, []);
  const [lastPath, setLastPath] = useState<string | null>(null);
  const [cutOpen, setCutOpen] = useState(false);
  // The cut dialog's weed controls, kept here so reopening the dialog keeps them. Per session, not
  // in the project: a weed tweak saved in the document would be an undoable edit that also makes
  // the open plan stale. (`ponytail:` an operator who always borders sets it once per launch;
  // upgrade by remembering it in the app's config beside presets.json.)
  const [cutWeed, setCutWeed] = useState<WeedDraft | null>(null);
  const [textOpen, setTextOpen] = useState(false);
  const [tracePath, setTracePath] = useState<string | null>(null);
  const [status, setStatus] = useState<ipc.CutStatus>(ipc.DISCONNECTED_STATUS);
  /** The machine's material presets, for the properties panel's control. Loaded here rather
   *  than in the panel because the document names the machine, and read for the *document's*
   *  machine rather than the connected one: a preset assigned to a Node is saved in the
   *  project, so the choice must not depend on which cutter happens to be plugged in. The cut
   *  dialog keeps its own list for the connected machine, which is what its settings apply to. */
  const [presets, setPresets] = useState<Preset[]>([]);

  const scene = useMemo(() => (doc ? buildScene(doc) : { nodes: [] }), [doc]);

  // Every successful snapshot gets the next revision, set in the same render as the document it
  // came with. A pending canvas preview retires by revision, not on any new document, because
  // queued commits can render an earlier one's snapshot while a later one is still on the wire.
  const revCounter = useRef(0);
  const [docRev, setDocRev] = useState(0);
  // Bumped when a load replaces the document. A snapshot asked for before that answers with the
  // document that left; rendered after the load, it showed that one under the next revision, which
  // also lifted the edit lock over the loaded document (Copilot on #301). So it is dropped, and
  // null says nothing rendered.
  const docGen = useRef(0);
  // Snapshots in the order they were asked for, and the latest that rendered. One answering after a
  // newer one rendered is older than what is on screen: shown under the next revision, it put the
  // canvas back to before an edit the newer one already showed (CodeRabbit on #301). It answers
  // with the revision on screen instead, which was read later and so holds everything it would
  // have: as null, a transform's preview stayed up over that newer scene (Copilot on #301).
  const snapshotsAsked = useRef(0);
  const snapshotShown = useRef(0);
  // Whether the last read of this document failed. After a load, edits stay locked until its
  // snapshot renders; when that read fails nothing else will render one, so the lock says to Reload
  // rather than that the document is still loading (silent-failure-hunter on #301).
  const [readFailed, setReadFailed] = useState(false);
  const refresh = useCallback(async (): Promise<number | null> => {
    const gen = docGen.current;
    const asked = ++snapshotsAsked.current;
    let json: string;
    try {
      json = (await ipc.snapshot()) as string;
    } catch (e) {
      if (gen === docGen.current) setReadFailed(true);
      throw e;
    }
    if (gen !== docGen.current) return null;
    if (asked < snapshotShown.current) return revCounter.current;
    snapshotShown.current = asked;
    const parsed = JSON.parse(json) as DocSnapshot;
    const rev = ++revCounter.current;
    setDoc(parsed);
    setDocRev(rev);
    setReadFailed(false);
    return rev;
  }, []);

  // ponytail: every command re-fetches the full snapshot instead of applying its returned
  // Delta locally with reconcile() — correct and simple while scenes stay tiny. Canvas gestures
  // (useCanvasInteraction) use applyOptimistic for live feedback, then also just re-fetch once
  // their commit lands; reconcile() stays unused until per-frame delta application is worth it.
  const run = useCallback(
    async (fn: () => Promise<unknown>) => {
      try {
        setError(null);
        await fn();
        await refresh();
        return true;
      } catch (e) {
        // No silent failures: every caught error is surfaced in the status bar. Device/cut
        // commands reject with a structured IpcError, not a string — ipcErrorMessage handles
        // both that and the plain-string rejections from the older doc-editing commands.
        setError(ipc.ipcErrorMessage(e));
        return false;
      }
    },
    [refresh],
  );

  // Returns the request so a caller that polls can wait for it: a tick that fires while the last
  // one is still in flight has to be skipped, and it cannot skip what it cannot see finish.
  const refreshDeviceState = useCallback(() => {
    return ipc
      .getDeviceState()
      .then(setStatus)
      .catch((e) => setError(ipc.ipcErrorMessage(e)));
  }, []);

  useEffect(() => {
    refresh().catch((e) => setError(ipc.ipcErrorMessage(e)));
    ipc
      .listMachines()
      .then((m) => setMachines(m as MachineProfile[]))
      .catch((e) => setError(ipc.ipcErrorMessage(e)));
  }, [refresh]);

  // Re-read when the document's machine changes, because presets are machine-scoped: a project
  // converted from a Cameo to a Puma must not keep offering Cameo materials. No machine set
  // means no list — there is nothing for a preset id to be scoped to yet.
  const docMachineId = doc?.machine?.id ?? null;
  useEffect(() => {
    if (docMachineId === null) {
      setPresets([]);
      return;
    }
    ipc
      .listPresets(docMachineId)
      .then((p) => setPresets(p as Preset[]))
      .catch((e) => setError(ipc.ipcErrorMessage(e)));
  }, [docMachineId]);

  // Mount-once device-event listener. Every event carries the status that held when it
  // was sent, so keeping the latest is the whole job — no event-kind interpreting, no
  // job-id filtering (one worker sends them in order over one channel), no poll.
  useEffect(() => {
    const unlisten = listen<ipc.DeviceEvent>("device-event", (e) => {
      setStatus(e.payload.status);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // Backend refuses to close the window mid-cut (main.rs's on_window_event calls
  // prevent_close and emits this instead) — ask the user, then force_quit if they
  // confirm. No-op on cancel, so the window just stays open.
  useEffect(() => {
    const unlisten = listen("cut-in-progress", () => {
      if (window.confirm("A cut is in progress — quit anyway?")) {
        ipc.forceQuit().catch((e) => setError(ipc.ipcErrorMessage(e)));
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  useEffect(() => {
    const ctx = canvasRef.current?.getContext("2d");
    if (ctx) rendererRef.current = new Canvas2DRenderer(ctx);
  }, []);

  // The last refresh warning a commit raised, so a later successful refresh can retract exactly it.
  const refreshWarning = useRef<string | null>(null);
  const expand = useCallback((ids: number[]) => (doc ? shapesUnder(doc.nodes, ids) : ids), [doc]);

  // After the renderer is constructed, so it exists before the hook's observer first fires.
  const interaction = useCanvasInteraction({
    canvasRef,
    rendererRef,
    scene,
    selected,
    setSelected,
    expand,
    artboard: doc?.artboard ?? null,
    // Not `run`: it reports one `false` for two failures that need opposite repairs. A refused
    // transform must put the shape back; one that landed but could not be re-read must keep it,
    // because the backend already holds the new geometry (silent-failure-hunter on #298).
    // A message raised after this edit was asked for stays: a refusal ahead of a queued edit would
    // otherwise vanish the moment the edit went out, before anyone saw it (silent-failure-hunter,
    // from #298).
    commit: async (moves, requestedAt): Promise<CommitOutcome> => {
      // Strictly older: WebKit's clock is coarse, and a refusal stamped in the same tick as a click
      // queued behind its commit was raised after that click, not before.
      if (errorAt.current < requestedAt) setError(null);
      try {
        await ipc.commitTransforms({ moves });
      } catch (e) {
        setError(ipc.ipcErrorMessage(e));
        return { kind: "refused" };
      }
      try {
        const snapshotRev = await refresh();
        // A refresh that worked makes an earlier "could not be refreshed" untrue, so that one
        // message goes even from a queued commit; a refusal stays (code-reviewer).
        setError((shown) => (shown !== null && shown === refreshWarning.current ? null : shown));
        return { kind: "applied", snapshotRev };
      } catch (e) {
        const warning = `Edit applied, but the canvas could not be refreshed: ${ipc.ipcErrorMessage(e)}`;
        refreshWarning.current = warning;
        setError(warning);
        return { kind: "applied", snapshotRev: null };
      }
    },
    sceneRev: docRev,
  });

  // Every other command that changes the document, refused while Open or Reload replaces it and
  // until the loaded one renders: each names ids or the root of the document on screen, which the
  // loaded one reuses, so a Delete pressed during a Reload removed what it gave those ids
  // (CodeRabbit on #301). Refused with a word, like the panel's locked controls.
  // The lock is read when an edit starts, and some then wait before they send (an import reads its
  // file first): a load waits for every edit started before it, or one could name the old root in
  // the new document (CodeRabbit on #301). `run` never rejects, so neither does the wait.
  const editsStarted = useRef(new Set<Promise<boolean>>());
  const { editsLockNow } = interaction;
  const lockReason = (lock: EditsLock) =>
    lock === null ? null
    : lock === "unread" && readFailed ? "the loaded document could not be read. Reload to try again"
    : "waiting for the document to load";
  // A refusal after the loaded document's read failed also reads it again: the lock lifts once a
  // snapshot renders, so the next try can go through without a Reload.
  const refuseEdit = useCallback(
    (lock: "loading" | "unread") => {
      if (lock === "loading" || !readFailed) {
        setError("Not applied: the document is still loading");
        return;
      }
      const refused = "Not applied: the loaded document could not be read. Reading it again";
      setError(refused);
      refresh().then(
        (rev) => {
          // Null: a newer load replaced the document meanwhile, and its own read speaks for it.
          if (rev === null) return;
          setError((shown) => (shown === refused ? "Not applied, but the loaded document is on screen now: try again" : shown));
        },
        (e) => {
          const failed = `Not applied: the loaded document could not be read: ${ipc.ipcErrorMessage(e)}. Reload to try again`;
          setError((shown) => (shown === refused ? failed : shown));
        },
      );
    },
    [readFailed, refresh, setError],
  );
  const edit = useCallback(
    (fn: () => Promise<unknown>) => {
      const lock = editsLockNow();
      if (lock !== null) {
        refuseEdit(lock);
        return Promise.resolve(false);
      }
      const started = run(fn);
      editsStarted.current.add(started);
      void started.finally(() => editsStarted.current.delete(started));
      return started;
    },
    [run, refuseEdit, editsLockNow],
  );

  // Inside `replaceDocument`, which has locked edits by then, so nothing joins the set while it waits.
  const loadDocument = (path: string) =>
    interaction.replaceDocument(async () => {
      await Promise.all(editsStarted.current);
      await ipc.loadProject({ path });
      docGen.current++;
      setReadFailed(false);
    });

  const { repaint } = interaction;
  useEffect(() => {
    const r = rendererRef.current;
    if (!r) return;
    r.setArtboard(doc?.artboard ?? null);
    r.setView(viewMatrix(interaction.view));
    // The hook draws the scene, so a redraw here cannot paint the committed scene over a live
    // gesture. `scene` and `selected` are listed because they are what it draws; `size` because a
    // resize clears the backing store and nothing else would repaint it.
    repaint();
  }, [scene, selected, doc, interaction.view, interaction.size, repaint]);

  // Clears selection only once the delete actually lands, so a failed delete leaves the
  // (still valid) selection in place, and a successful one can't leave stale ids around to
  // error out a later transform.
  const deleteSelected = useCallback(() => {
    edit(() => ipc.deleteNodes({ ids: selected })).then((ok) => {
      if (ok) setSelected([]);
    });
  }, [edit, selected]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      const typing = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA");
      if ((e.key === "Delete" || e.key === "Backspace") && !typing) {
        if (selected.length === 0) return;
        e.preventDefault();
        deleteSelected();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z" && !typing) {
        e.preventDefault();
        edit(() => (e.shiftKey ? ipc.redo() : ipc.undo()));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected, edit, deleteSelected]);

  const root = doc?.root ?? 0;
  // Bounds of a single selection in a given scene. The fields read the effective scene, which holds
  // an unread or in-flight commit's preview, so they show (and compute from) where the shape is now.
  const boundsIn = (s: Scene) =>
    selected.length === 1 ? (s.nodes.find((n) => n.id === selected[0])?.bounds ?? null) : null;
  const selectedBounds = boundsIn(interaction.effectiveScene);

  // Through the hook rather than straight to the backend: a field edit then waits behind a
  // commit on the wire and builds its matrix from the geometry as it stands when it is sent, not
  // from a position the shape has already left (Copilot on #298).
  const commitAxis = (axis: "x" | "y", v: number) =>
    interaction.transformWith(axis, selected, (s): Matrix | null => {
      const b = boundsIn(s);
      return b ? [1, 0, 0, 1, axis === "x" ? v - b.x : 0, axis === "y" ? v - b.y : 0] : null;
    });

  // Scale about the bounds origin (x for width, y for height) so the opposite edge stays
  // put: translate(origin) · scale(s) · translate(-origin), i.e. [s,0,0,1, x-s*x, 0] for
  // width and [1,0,0,s, 0, y-s*y] for height.
  const commitScale = (axis: "w" | "h", v: number) =>
    interaction.transformWith(axis, selected, (s): Matrix | null => {
      const b = boundsIn(s);
      if (!b) return null;
      const size = axis === "w" ? b.w : b.h;
      if (size <= 0 || v <= 0) return null;
      const k = v / size;
      return axis === "w" ? [k, 0, 0, 1, b.x - k * b.x, 0] : [1, 0, 0, k, 0, b.y - k * b.y];
    });

  // One unit per selected id that no other selected id contains, bounded by every shape it moves,
  // so a Group lines up as a drag would move it. Read from the scene the hook passes in, which holds
  // any unread commit's preview. Listed in document order (the scene's, by each unit's first shape)
  // rather than the order they were clicked, since that is what breaks a distribute tie.
  // Memoised: a pointer move re-renders App for the cursor readout, and rebuilding these scanned
  // every node in the document per move (CodeRabbit on #301).
  const units = useMemo(() => (doc ? outermost(doc.nodes, selected) : []), [doc, selected]);
  // Keyed like the fields: a newer click of the same kind on the same axis and selection replaces a
  // queued one, but "Align top" does not replace a queued "Align left", and a distribute does not
  // replace an align, since each is a different request (code-reviewer).
  // Sorted, because `outermost` keeps click order and reselecting a piece reorders it: the same
  // selection must make the same key, or a newer click runs after the older one instead of
  // replacing it (Copilot on #301).
  const alignKey = (kind: "align" | "distribute", axis: Axis) =>
    `${kind}:${axis}:${[...units].sort((p, q) => p - q).join(",")}`;
  // A queued click keeps the ids it was made on but reads the tree and the bed as they stand when it
  // is sent (`SendTime`): the commit ahead of it may have changed a Group's shapes, and a machine
  // switch in between centred a piece on the old bed (Copilot on #301).
  const align = (mode: AlignMode) => {
    const ids = units;
    // What the click meant, kept for when it is sent: lining the pieces up with each other, or one
    // piece with the bed. A unit deleted while the click waited would otherwise turn the first into
    // the second, and send the piece left over to the edge of the mat.
    const toEachOther = unitCount >= 2;
    interaction.transformEach(alignKey("align", AXIS[mode]), (s, now) => {
      const sent = unitsOf(s, ids, now.expand);
      return sent.length >= 2 === toEachOther ? alignMoves(sent, mode, now.artboard) : [];
    });
  };
  const distribute = (axis: Axis) => {
    const ids = units;
    interaction.transformEach(alignKey("distribute", axis), (s, now) => distributeMoves(unitsOf(s, ids, now.expand), axis));
  };
  // Counted from the scene the moves are computed from, not from the selection: an empty Group
  // is a selected id with nothing to line up, and counting it enabled a distribute that then had
  // two units and did nothing, or sent a lone shape to the artboard (silent-failure-hunter).
  const shownUnits = useMemo(() => unitsOf(interaction.effectiveScene, units, expand), [interaction.effectiveScene, units, expand]);
  const unitCount = shownUnits.length;
  const distributeBlocked = useMemo(
    () => ({ x: distributeBlock(shownUnits, "x"), y: distributeBlock(shownUnits, "y") }),
    [shownUnits],
  );

  const cutLineType = doc ? selectionCutLineType(doc.nodes, selected) : null;

  const setCutLineType = (value: CutLineTypeJson) => {
    if (selected.length === 0) return;
    edit(() => ipc.setCutLineType({ ids: selected, value }));
  };

  // The selection's own assignment, and what it resolves to. Both, because `Inherit` alone
  // does not tell an operator which material the blade will be set for.
  const materialPreset = doc ? selectionAssignment(doc.nodes, selected) : undefined;
  // Across the whole selection, not just its first node: two shapes that both say `Inherit`
  // under different Layers agree on their *local* value and resolve differently, and labelling
  // that with the first one's material misreports what a bulk edit is about to replace.
  // Memoized on the document: this walks the whole tree, and `App` re-renders for unrelated
  // reasons — a device-status event arrives per progress tick during a live cut, and doing an
  // O(nodes) walk on each of those is work nobody asked for.
  const materialsByNode = useMemo(
    () => (doc ? effectiveMaterials(doc.nodes, doc.root) : {}),
    [doc],
  );
  const effectiveMaterial = summariseEffectiveMaterial(materialsByNode, selected);

  const setMaterialPreset = (value: ipc.PresetAssignmentJson) => {
    if (selected.length === 0) return;
    edit(() => ipc.setMaterialPreset({ ids: selected, value }));
  };

  // A successful boolean op removes the source nodes and adds a result node — selecting
  // the removed ids would error the next transform with NotFound, so read the result id
  // straight out of the returned Delta's Add op and select that instead (or clear
  // selection if the shape ever comes back without one).
  const onBooleanOp = useCallback(
    (op: BoolOp) => {
      edit(async () => {
        const delta = (await ipc.booleanOp({ ids: selected, op })) as NodeOpJson[];
        const added = delta.find((o): o is Extract<NodeOpJson, { Add: unknown }> => "Add" in o);
        setSelected(added ? [added.Add.node.id] : []);
      });
    },
    [edit, selected],
  );

  const onImportFile = (file: File) => {
    edit(async () => {
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      const [, skipped] = (await ipc.importSvg({ bytes, parent: root })) as [unknown, string[]];
      if (skipped.length > 0) setError(`Imported with ${skipped.length} element(s) skipped: ${skipped.join(", ")}`);
    });
  };

  const onTrace = () =>
    run(async () => {
      const p = await ipc.pickImagePath();
      if (p) setTracePath(p);
    });

  const onTraceInsert = (svg: string) => {
    edit(async () => {
      const bytes = Array.from(new TextEncoder().encode(svg));
      const [, skipped] = (await ipc.importSvg({ bytes, parent: root })) as [unknown, string[]];
      if (skipped.length > 0) setError(`Inserted with ${skipped.length} element(s) skipped: ${skipped.join(", ")}`);
    });
    setTracePath(null);
  };

  return (
    <div
      style={{
        display: "grid",
        gridTemplateRows: "auto 1fr auto",
        gridTemplateColumns: "auto 1fr 280px",
        height: "100%",
      }}
    >
      <div style={{ gridColumn: "1 / -1" }}>
        <TopBar
          machines={machines}
          currentMachineId={doc?.machine?.id ?? null}
          onSelectMachine={(id) => edit(() => ipc.setMachine({ machineId: id }))}
          onSave={() =>
            run(async () => {
              const p = await ipc.pickSavePath();
              if (p) {
                await ipc.saveProject({ path: p });
                setLastPath(p);
              }
            })
          }
          onOpen={() =>
            run(async () => {
              const p = await ipc.pickOpenPath();
              if (p) {
                // Held for the whole load, not cleared before it: the backend has replaced the
                // document by the time loadProject resolves, and a commit settling in between would
                // drain into it (CodeRabbit on #301), as would an edit made while it loads.
                await loadDocument(p);
                setLastPath(p);
                setSelected([]); // loaded doc may not contain the old ids
                interaction.requestFit();
              }
            })
          }
          onReload={() =>
            run(async () => {
              await loadDocument(lastPath!);
              setSelected([]);
              interaction.requestFit();
            })
          }
          canReload={lastPath !== null}
          onUndo={() => edit(() => ipc.undo())}
          onRedo={() => edit(() => ipc.redo())}
          onImportFile={onImportFile}
          onCut={() => setCutOpen(true)}
          onTrace={onTrace}
        />
      </div>
      <ToolRail
        tool={tool}
        selectionCount={selected.length}
        onSelectTool={setTool}
        onAddRect={() => edit(() => ipc.addPrimitive({ parent: root, kind: { Rect: { w: 20, h: 20 } } }))}
        onAddEllipse={() => edit(() => ipc.addPrimitive({ parent: root, kind: { Ellipse: { rx: 10, ry: 10 } } }))}
        onAddText={() => setTextOpen(true)}
        onBoolean={onBooleanOp}
        onDelete={deleteSelected}
      />
      {/* The wrapper is what lets the canvas fill its grid cell: a canvas sized 100% directly in
          the grid adds its intrinsic 300×150 to the track minimum and never shrinks. `data-view`
          lets e2e turn document mm into page px, since the canvas's pixels are unreadable there. */}
      <div style={{ position: "relative", minWidth: 0, minHeight: 0, overflow: "hidden" }}>
        <canvas
          ref={canvasRef}
          data-testid="design-canvas"
          data-view={`${interaction.view.scale} ${interaction.view.tx} ${interaction.view.ty}`}
          style={{ position: "absolute", inset: 0, width: "100%", height: "100%", display: "block", background: "var(--workspace)", touchAction: "none" }}
          {...interaction.handlers}
        />
      </div>
      <div style={{ display: "grid", gridTemplateRows: "1fr 1fr", borderLeft: "1px solid var(--border)", minHeight: 0 }}>
        <LayersPanel
          doc={doc}
          selected={selected}
          onSelect={(id, shiftKey) => setSelected((prev) => (shiftKey ? toggleId(prev, id) : [id]))}
        />
        <PropertiesPanel
          bounds={selectedBounds}
          onChangeX={(v) => commitAxis("x", v)}
          onChangeY={(v) => commitAxis("y", v)}
          onChangeW={(v) => commitScale("w", v)}
          onChangeH={(v) => commitScale("h", v)}
          unitCount={unitCount}
          distributeBlocked={distributeBlocked}
          editsLocked={lockReason(interaction.editsLock)}
          onAlign={align}
          onDistribute={distribute}
          cutLineType={cutLineType}
          onChangeCutLineType={setCutLineType}
          materialPreset={materialPreset}
          effectiveMaterial={effectiveMaterial}
          presets={presets}
          onChangeMaterialPreset={setMaterialPreset}
        />
      </div>
      <div style={{ gridColumn: "1 / -1" }}>
        <StatusBar
          machine={doc?.machine ?? null}
          artboard={doc?.artboard ?? null}
          error={error}
          status={status}
          zoomPercent={doc ? zoomPercent(interaction.view) : null}
          cursor={interaction.cursor}
        />
      </div>
      {cutOpen && doc ? (
        <CutDialog
          scene={scene}
          artboard={doc.artboard}
          docMachineId={doc.machine?.id ?? null}
          status={status}
          refreshDeviceState={refreshDeviceState}
          onConvertMachine={(machineId) => edit(() => ipc.setMachine({ machineId }))}
          onError={setError}
          onClose={() => setCutOpen(false)}
          weed={cutWeed}
          onWeedChange={setCutWeed}
        />
      ) : null}
      {tracePath !== null ? (
        <TraceDialog path={tracePath} onInsert={onTraceInsert} onClose={() => setTracePath(null)} />
      ) : null}
      {textOpen ? (
        <TextDialog
          onInsert={(family) => {
            setTextOpen(false);
            // ponytail: content and size are fixed until #33 grows this dialog into real
            // text editing; the family is the only choice the backend can act on today.
            edit(() => ipc.addText({ parent: root, family, sizeMm: 10, text: "Text" }));
          }}
          onClose={() => setTextOpen(false)}
        />
      ) : null}
    </div>
  );
}
