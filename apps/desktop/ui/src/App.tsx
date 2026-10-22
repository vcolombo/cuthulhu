// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { listen } from "@tauri-apps/api/event";
import { Cpu, Layers, SlidersHorizontal } from "lucide-react";
import * as ipc from "./ipc";
import { Toaster } from "./components/ui/sonner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./components/ui/tabs";
import { TooltipProvider } from "./components/ui/tooltip";
import { useActiveCutter } from "./hooks/useActiveCutter";
import { useDevices } from "./hooks/useDevices";
import { useDocument } from "./hooks/useDocument";
import { useJob } from "./hooks/useJob";
import { useUnits } from "./hooks/useUnits";
import { DevicesTab } from "./panels/DevicesTab";
import { Canvas2DRenderer } from "./render/Canvas2DRenderer";
import { unionBounds, type Affine6, type Bounds, type Scene, type ShapeGeom } from "./render/hittest";
import { pathBounds } from "./render/pathdata";
import { IDENTITY, compose, transformBounds } from "./render/affine";
import { outermost, shapesUnder, toggleId } from "./interaction/marquee";
import { AXIS, alignMoves, distributeBlock, distributeMoves, type AlignMode, type Axis, type Unit } from "./interaction/align";
import type { Matrix } from "./interaction/transform";
import { useCanvasInteraction, type CommitOutcome, type EditsLock } from "./interaction/useCanvasInteraction";
import { viewMatrix, zoomPercent, type Inset } from "./interaction/viewport";
import { TopBar } from "./panels/TopBar";
import { ToolRail } from "./panels/ToolRail";
import { LayersPanel } from "./panels/LayersPanel";
import { PropertiesPanel } from "./panels/PropertiesPanel";
import { RULER_PX, ZOOM_BAR_CLEAR_PX, Rulers, ZoomControls } from "./panels/CanvasChrome";
import { selectionCutLineType, type CutLineTypeJson } from "./panels/cutLineType";
import { effectiveMaterials, selectionAssignment, summariseEffectiveMaterial } from "./panels/materialPreset";
import { StatusBar } from "./panels/StatusBar";
import { JobPanel } from "./cut/job/JobPanel";
import { jobIsActive, jobSurface, placeOf } from "./cut/job/model";
import { JobStatusChip, SwapBar, SwapChip } from "./cut/job/SwapBar";
import { SendSheet } from "./cut/send/SendSheet";
import type { WeedDraft } from "./cut/viewmodel";
import { TraceDialog } from "./trace/TraceDialog";
import { TextDialog } from "./text/TextDialog";
import { useTheme } from "./useTheme";
import { colorModeVinyl, selectionHasFaintVinyl } from "./render/vinyl";
import { matBounds, matKindsFor, offeredKind, clampRollLength, ROLL_DEFAULT_MM, type MatKind } from "./render/mat";

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

export type StyleJson = { stroke: number | null; fill: number | null };

export type DocNode = {
  id: number;
  kind: NodeKindJson;
  transform: Affine6;
  style: StyleJson;
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
      nodes.push({
        id: n.id,
        bounds: transformBounds(world, local),
        local,
        shape: shapeGeom(n.kind.Shape),
        world,
        vinyl: colorModeVinyl(n.style),
        cut: n.cut_line_type !== "NoCut",
      });
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

function countShapes(doc: DocSnapshot | null): number {
  if (!doc) return 0;
  return Object.values(doc.nodes).filter((node) => typeof node.kind === "object" && "Shape" in node.kind).length;
}

function selectionBounds(scene: Scene, selected: number[]): Bounds | null {
  const boxes = scene.nodes.filter((node) => selected.includes(node.id)).map((node) => node.bounds);
  if (boxes.length === 0) return null;
  const x = Math.min(...boxes.map((box) => box.x));
  const y = Math.min(...boxes.map((box) => box.y));
  const right = Math.max(...boxes.map((box) => box.x + box.w));
  const bottom = Math.max(...boxes.map((box) => box.y + box.h));
  return { x, y, w: right - x, h: bottom - y };
}

export function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<Canvas2DRenderer | null>(null);
  const theme = useTheme();
  // The canvas hook does not exist yet on the first line. Edits read it when they start, which is
  // after this render has stored the real lock.
  const editsLockNow = useRef<() => EditsLock>(() => "loading");
  const docState = useDocument(editsLockNow);
  const {
    doc, selected, setSelected, machines, presets, setError, errorAt, lastPath, setLastPath,
    docRev, history, refresh, run, edit, lockReason, waitForEdits, noteLoaded, root,
  } = docState;

  // A preferences failure is the same toast as a document one. It is not cleared when the fault
  // goes away: a later edit owns that, and wiping it would hide a message that arrived in between.
  useEffect(() => {
    if (theme.fault) setError(theme.fault.message);
  }, [theme.fault, setError]);

  const [tool, setTool] = useState("select");
  const [cutOpen, setCutOpen] = useState(false);
  const [swapCollapsed, setSwapCollapsed] = useState(false);
  const [expandJob, setExpandJob] = useState(0);
  // The cut dialog's weed controls, kept here so reopening the dialog keeps them. Per session, not
  // in the project: a weed tweak saved in the document would be an undoable edit that also makes
  // the open plan stale. (`ponytail:` an operator who always borders sets it once per launch;
  // upgrade by remembering it in preferences.json, which already holds appearance.)
  const [cutWeed, setCutWeed] = useState<WeedDraft | null>(null);
  const [textOpen, setTextOpen] = useState(false);
  const [tracePath, setTracePath] = useState<string | null>(null);
  const [rightTab, setRightTab] = useState("layers");
  // The mat is view state. Saving it in the project is #199; a reload starts back on 12×12.
  const [matKind, setMatKind] = useState<MatKind>("12x12");
  const [rollLengthMm, setRollLengthMm] = useState(ROLL_DEFAULT_MM);
  // Rulers match the approved canvas. The control turns them off; the fit inset follows.
  const [rulers, setRulers] = useState(true);

  const scene = useMemo(() => (doc ? buildScene(doc) : { nodes: [] }), [doc]);
  const mat = useMemo(() => {
    if (!doc) return null;
    const width = doc.machine?.width_mm ?? doc.artboard.w;
    return matBounds(offeredKind(matKind, doc.machine?.id ?? null), width, rollLengthMm);
  }, [doc, matKind, rollLengthMm]);
  const contentBounds = useMemo(
    () => (scene.nodes.length > 0 ? unionBounds(scene.nodes.map((n) => n.bounds)) : null),
    [scene],
  );
  const inset = useMemo<Inset>(
    () => ({
      left: rulers ? RULER_PX : 0,
      top: rulers ? RULER_PX : 0,
      right: 0,
      // The bar sits on the canvas, so the mat is fitted to the area above it.
      bottom: ZOOM_BAR_CLEAR_PX,
    }),
    [rulers],
  );
  const job = useJob(setError);
  const status = job.status;
  const { devices, hosts, listError: deviceListError, refresh: refreshDevices } = useDevices();
  const { unit, setUnit } = useUnits(setError);
  const designFor = useCallback((machineId: string) => edit(() => ipc.setMachine({ machineId })), [edit]);
  const cutter = useActiveCutter({
    profileId: doc?.machine?.id ?? null,
    shapeCount: countShapes(doc),
    machines,
    designFor,
    refreshDeviceState: job.refreshDeviceState,
    onError: setError,
  });
  useEffect(() => {
    job.attach(cutter.device);
  }, [cutter.device, job.attach]);

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
    mat,
    contentBounds,
    inset,
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
  // Stored for the next edit. The document hook reads it when an edit starts, which is after this.
  editsLockNow.current = interaction.editsLockNow;
  const fitContentNow = useRef(interaction.requestFitContent);
  fitContentNow.current = interaction.requestFitContent;

  // Inside `replaceDocument`, which has locked edits by then, so nothing joins the set while it waits.
  const loadDocument = (path: string) =>
    interaction.replaceDocument(async () => {
      await waitForEdits();
      await ipc.loadProject({ path });
      noteLoaded();
    });

  const { repaint } = interaction;
  useEffect(() => {
    const r = rendererRef.current;
    if (!r) return;
    r.setMat(mat);
    r.setGridUnit(unit);
    r.setView(viewMatrix(interaction.view));
    // The hook draws the scene, so a redraw here cannot paint the committed scene over a live
    // gesture. `scene` and `selected` are listed because they are what it draws; `size` because a
    // resize clears the backing store and nothing else would repaint it.
    repaint();
    // `theme.resolved` is what the canvas reads out of the CSS variables. A theme
    // change does not touch the scene, so without it the selection stays in the
    // previous theme's colour until the next gesture.
  }, [scene, selected, doc, mat, unit, interaction.view, interaction.size, repaint, theme.resolved]);

  // Clears selection only once the delete actually lands, so a failed delete leaves the
  // (still valid) selection in place, and a successful one can't leave stale ids around to
  // error out a later transform.
  const deleteSelected = useCallback(() => {
    edit(() => ipc.deleteNodes({ ids: selected })).then((ok) => {
      if (ok) setSelected([]);
    });
  }, [edit, selected]);

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
    void edit(async () => {
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      const [, skipped] = (await ipc.importSvg({ bytes, parent: root })) as [unknown, string[]];
      if (skipped.length > 0) setError(`Imported with ${skipped.length} element(s) skipped: ${skipped.join(", ")}`);
    }).then((ok) => {
      if (ok) fitContentNow.current();
    });
  };

  const onTrace = () =>
    run(async () => {
      const p = await ipc.pickImagePath();
      if (p) setTracePath(p);
    });

  const onTraceInsert = (svg: string) => {
    void edit(async () => {
      const bytes = Array.from(new TextEncoder().encode(svg));
      const [, skipped] = (await ipc.importSvg({ bytes, parent: root })) as [unknown, string[]];
      if (skipped.length > 0) setError(`Inserted with ${skipped.length} element(s) skipped: ${skipped.join(", ")}`);
    }).then((ok) => {
      if (ok) fitContentNow.current();
    });
    setTracePath(null);
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      const typing = t !== null && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable || t.closest("[role='dialog'], [role='menu']") !== null);
      if (typing) return;
      const key = e.key.toLowerCase();
      const mod = e.metaKey || e.ctrlKey;
      if (e.key === "Delete" || e.key === "Backspace") {
        if (selected.length === 0) return;
        e.preventDefault();
        deleteSelected();
        return;
      }
      if (mod && key === "z") {
        e.preventDefault();
        if (e.shiftKey) {
          if (history.can_redo) edit(() => ipc.redo());
        } else if (history.can_undo) {
          edit(() => ipc.undo());
        }
        return;
      }
      if (mod && key === "u") {
        e.preventDefault();
        if (selected.length >= 2) onBooleanOp("Union");
        return;
      }
      // Option on macOS and AltGr change `e.key` (Option+S is "ß"), so the physical key is `e.code`.
      if (mod && e.altKey && e.code === "KeyS") {
        e.preventDefault();
        if (selected.length >= 2) onBooleanOp("Subtract");
        return;
      }
      if (mod && e.altKey && e.code === "KeyI") {
        e.preventDefault();
        if (selected.length >= 2) onBooleanOp("Intersect");
        return;
      }
      if (mod && e.altKey && e.code === "KeyE") {
        e.preventDefault();
        if (selected.length >= 2) onBooleanOp("Exclude");
        return;
      }
      if (mod) return;
      if (e.shiftKey && key === "t") {
        e.preventDefault();
        onTrace();
        return;
      }
      if (key === "v") setTool("select");
      else if (key === "r") {
        void edit(() => ipc.addPrimitive({ parent: root, kind: { Rect: { w: 20, h: 20 } } })).then((ok) => {
          if (ok) fitContentNow.current();
        });
      } else if (key === "e") {
        void edit(() => ipc.addPrimitive({ parent: root, kind: { Ellipse: { rx: 10, ry: 10 } } })).then((ok) => {
          if (ok) fitContentNow.current();
        });
      } else if (key === "t") setTextOpen(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected, edit, deleteSelected, history.can_undo, history.can_redo, onBooleanOp, onTrace, root]);

  const cutting = status.phase === "Sending" || status.phase === "AwaitingColorSwap" || status.phase === "AwaitingConfirmation";
  const selectionSize = selectionBounds(interaction.effectiveScene, selected);

  const surface = jobSurface(status, job.tracking);
  // Leaving the swap drops the chip. Set during render so the bar and the chip never both show.
  if (surface !== "swap" && swapCollapsed) setSwapCollapsed(false);
  const place = placeOf(cutter.device, machines, hosts);
  const jobRunning = job.tracking && jobIsActive(status.phase);
  const viewJob = () => {
    if (surface === "swap") setSwapCollapsed(false);
    setExpandJob((n) => n + 1);
  };
  useEffect(() => {
    if (expandJob === 0) return;
    const frame = requestAnimationFrame(() => {
      document.getElementById("job-surface")?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [expandJob]);

  const reportPick = (info: ipc.DeviceInfo) => {
    void cutter.pick(info).catch((e) => setError(ipc.ipcErrorMessage(e)));
  };

  return (
    <TooltipProvider>
    {/* Inline height and grid tracks, not `h-full`/`flex-1`: a percentage height that does not
        resolve collapses the middle row to 0, the canvas reports a 1px bitmap, and fit never runs. */}
    <div
      className="bg-background text-foreground"
      style={{
        display: "grid",
        // A fixed rail. An `auto` first column absorbs the min-content of the bars that span
        // every column, and the canvas then shrinks until a handle's hit radius covers the
        // middle of a small shape.
        gridTemplateRows: "auto minmax(0, 1fr) auto",
        gridTemplateColumns: "64px minmax(0, 1fr) 320px",
        height: "100%",
        minHeight: 0,
      }}
    >
      <div style={{ gridColumn: "1 / -1", minWidth: 0 }}>
      <TopBar
        canUndo={history.can_undo}
        canRedo={history.can_redo}
        onUndo={() => edit(() => ipc.undo())}
        onRedo={() => edit(() => ipc.redo())}
        onImportFile={onImportFile}
        onTrace={onTrace}
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
        unit={unit}
        onUnit={setUnit}
        appearance={theme.appearance}
        appearanceReady={theme.ready}
        onAppearance={theme.setAppearance}
        device={cutter.device}
        profileId={doc?.machine?.id ?? null}
        machines={machines}
        devices={devices}
        hosts={hosts}
        phase={status.phase}
        onPick={reportPick}
        onRefreshDevices={() => {
          void refreshDevices();
        }}
        onSend={() => setCutOpen(true)}
      />
      </div>
      <ToolRail
        tool={tool}
        selectionCount={selected.length}
        onSelectTool={setTool}
        onAddRect={() => {
          void edit(() => ipc.addPrimitive({ parent: root, kind: { Rect: { w: 20, h: 20 } } })).then((ok) => {
            if (ok) interaction.requestFitContent();
          });
        }}
        onAddEllipse={() => {
          void edit(() => ipc.addPrimitive({ parent: root, kind: { Ellipse: { rx: 10, ry: 10 } } })).then((ok) => {
            if (ok) interaction.requestFitContent();
          });
        }}
        onAddText={() => setTextOpen(true)}
        onTrace={onTrace}
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
        {rulers ? <Rulers view={interaction.view} size={interaction.size} unit={unit} /> : null}
        {doc ? (
          <ZoomControls
            zoomPercent={zoomPercent(interaction.view)}
            rulers={rulers}
            onZoomOut={interaction.zoomOut}
            onZoomIn={interaction.zoomIn}
            onFitMat={interaction.requestFit}
            onFitContent={interaction.requestFitContent}
            onToggleRulers={() => setRulers((on) => !on)}
          />
        ) : null}
      </div>
      <aside className="flex min-h-0 flex-col border-l border-border bg-panel">
        <Tabs value={rightTab} onValueChange={setRightTab} className="flex min-h-0 flex-1 flex-col gap-0">
          <TabsList className="m-2 grid h-8 grid-cols-3 bg-background p-0.5">
            <TabsTrigger value="layers" className="h-7 gap-1.5 text-xs"><Layers className="size-3.5" aria-hidden />Layers</TabsTrigger>
            <TabsTrigger value="properties" className="h-7 gap-1.5 text-xs"><SlidersHorizontal className="size-3.5" aria-hidden />Properties</TabsTrigger>
            <TabsTrigger value="devices" className="h-7 gap-1.5 text-xs">
              <Cpu className="size-3.5" aria-hidden />
              Devices
              {cutting ? <span className="size-1.5 rounded-full bg-success" aria-hidden /> : null}
            </TabsTrigger>
          </TabsList>
          <TabsContent value="layers" className="mt-0 min-h-0 flex-1 overflow-auto">
            <LayersPanel
              doc={doc}
              selected={selected}
              onSelect={(id, shiftKey) => setSelected((prev) => (shiftKey ? toggleId(prev, id) : [id]))}
            />
          </TabsContent>
          <TabsContent value="properties" className="mt-0 min-h-0 flex-1 overflow-auto">
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
              machines={machines}
              machineId={doc?.machine?.id ?? null}
              onDesignFor={designFor}
              faintVinyl={doc ? selectionHasFaintVinyl(doc.nodes, selected) : false}
            />
          </TabsContent>
          <TabsContent value="devices" className="mt-0 min-h-0 flex-1 overflow-auto">
            <DevicesTab devices={devices} hosts={hosts} listError={deviceListError} active={cutter.device} machines={machines} onPick={reportPick} />
          </TabsContent>
        </Tabs>
        {surface !== "hidden" && surface !== "swap" ? (
          <JobPanel
            key={surface}
            status={status}
            snapshot={job.snapshot}
            surface={surface}
            place={place}
            theme={theme.resolved}
            offlineHost={job.offlineHost}
            offlineSeconds={job.offlineSeconds}
            onCancel={job.cancel}
            onResume={job.resume}
            onConfirm={job.confirmPassDone}
            onStartAgain={() => setCutOpen(true)}
            onDismiss={job.dismiss}
            expandToken={expandJob}
          />
        ) : null}
      </aside>
      <div style={{ gridColumn: "1 / -1", minWidth: 0 }} className="flex min-w-0 flex-col">
      {surface === "swap" && !swapCollapsed ? (
        <SwapBar
          status={status}
          snapshot={job.snapshot}
          cutter={place.cutter}
          theme={theme.resolved}
          offlineHost={job.offlineHost}
          offlineSeconds={job.offlineSeconds}
          onCancel={job.cancel}
          onResume={job.resume}
          onConfirm={job.confirmPassDone}
          onCollapse={() => setSwapCollapsed(true)}
        />
      ) : null}
      <StatusBar
        device={cutter.device}
        profile={doc?.machine ?? null}
        machines={machines}
        hosts={hosts}
        mat={doc && mat ? {
          kind: offeredKind(matKind, doc.machine?.id ?? null),
          kinds: matKindsFor(doc.machine?.id ?? null),
          rollLengthMm,
          onKind: setMatKind,
          onRollLength: (mm) => setRollLengthMm(clampRollLength(mm)),
        } : null}
        status={status}
        zoomPercent={doc ? zoomPercent(interaction.view) : null}
        cursor={interaction.cursor}
        selection={selectionSize}
        unit={unit}
        hideJobEcho={surface !== "hidden"}
        trailing={surface === "swap" && swapCollapsed ? (
          <SwapChip
            status={status}
            snapshot={job.snapshot}
            theme={theme.resolved}
            offline={job.offlineHost !== null}
            onExpand={viewJob}
          />
        ) : jobRunning ? (
          <JobStatusChip
            status={status}
            snapshot={job.snapshot}
            theme={theme.resolved}
            offline={job.offlineHost !== null}
            onActivate={viewJob}
          />
        ) : null}
      />
      </div>
    </div>
      {cutOpen && doc ? (
        <SendSheet
          scene={scene}
          artboard={doc.artboard}
          docMachineId={doc.machine?.id ?? null}
          machines={machines}
          status={status}
          refreshDeviceState={job.refreshDeviceState}
          connected={cutter.device}
          onConnect={cutter.pick}
          onDisconnect={cutter.disconnect}
          onReconnect={cutter.reconnect}
          onConvertMachine={designFor}
          onError={setError}
          onClose={() => setCutOpen(false)}
          onJobAccepted={job.remember}
          weed={cutWeed}
          onWeedChange={setCutWeed}
          theme={theme.resolved}
          unit={unit}
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
            edit(() => ipc.addText({ parent: root, family, sizeMm: 10, text: "Text" })).then((ok) => {
              if (ok) interaction.requestFitContent();
            });
          }}
          onClose={() => setTextOpen(false)}
        />
      ) : null}
    {/* Top-right, and no wider than the viewport. A bottom-centred toast sits on the
        sheet footer and, wider than the window, is cut off at the left edge. */}
    <Toaster
      position="top-right"
      theme={theme.resolved}
      offset={{ top: "148px", right: "16px" }}
      mobileOffset={{ top: "16px", right: "16px" }}
      style={{ "--width": "min(356px, calc(100vw - 32px))" } as CSSProperties}
    />
    </TooltipProvider>
  );
}
