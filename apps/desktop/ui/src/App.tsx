// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import * as ipc from "./ipc";
import { Canvas2DRenderer } from "./render/Canvas2DRenderer";
import { hitTest, type Scene } from "./render/hittest";
import { applyOptimistic, dragMatrix, type Matrix, type Pt } from "./interaction/transform";
import { TopBar } from "./panels/TopBar";
import { ToolRail } from "./panels/ToolRail";
import { LayersPanel } from "./panels/LayersPanel";
import { PropertiesPanel } from "./panels/PropertiesPanel";
import { StatusBar } from "./panels/StatusBar";

// Shapes mirroring the Rust `document` crate's serde JSON. Loose but sufficient for the
// paths this UI actually reads — see crates/document/src/{node,delta,machine}.rs.
export type Affine6 = [number, number, number, number, number, number];
export type BoolOp = "Union" | "Subtract" | "Intersect" | "Exclude";

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
  children: number[];
};

export type MachineProfile = { id: string; name: string; width_mm: number; height_mm: number };

export type DocSnapshot = {
  nodes: Record<string, DocNode>;
  root: number;
  artboard: { x: number; y: number; w: number; h: number };
  machine: MachineProfile | null;
};

const PROJECT_PATH = "cuthulhu-project.cut"; // ponytail: fixed save path until tauri-plugin-dialog wires a file picker

function toggleId(ids: number[], id: number): number[] {
  return ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id];
}

function shapeBounds(kind: ShapeKindJson) {
  if ("Rect" in kind) return { x: 0, y: 0, w: kind.Rect.w, h: kind.Rect.h };
  if ("Ellipse" in kind) {
    // Canonical convention (see crates/document/src/commands.rs shape_to_path): an
    // Ellipse's local space is centered at (rx, ry), bounds 0..2rx / 0..2ry.
    return { x: 0, y: 0, w: kind.Ellipse.rx * 2, h: kind.Ellipse.ry * 2 };
  }
  // ponytail: Text nodes are converted server-side into a Path before insertion (add_text
  // mints a Path node), and precise Path bounds need a bbox pass over the outline — a
  // placeholder box covers both cases until that lands.
  return { x: 0, y: 0, w: 10, h: 10 };
}

const IDENTITY_AFFINE6: Affine6 = [1, 0, 0, 1, 0, 0];

// Mirrors crates/geometry/src/affine.rs's Affine::then: "self.then(other) = apply self,
// then other". Used to accumulate each node's local transform into its ancestors' world
// transform on the way down the tree (self=node.transform, other=parentWorld).
function composeThen(self: Affine6, other: Affine6): Affine6 {
  const [a1, b1, c1, d1, e1, f1] = self;
  const [a2, b2, c2, d2, e2, f2] = other;
  return [
    a2 * a1 + c2 * b1,
    b2 * a1 + d2 * b1,
    a2 * c1 + c2 * d1,
    b2 * c1 + d2 * d1,
    a2 * e1 + c2 * f1 + e2,
    b2 * e1 + d2 * f1 + f2,
  ];
}

function applyAffine(m: Affine6, x: number, y: number): Pt {
  const [a, b, c, d, e, f] = m;
  return { x: a * x + c * y + e, y: b * x + d * y + f };
}

function buildScene(doc: DocSnapshot): Scene {
  const nodes: Scene["nodes"] = [];
  const walk = (id: number, parentWorld: Affine6) => {
    const n = doc.nodes[id];
    if (!n) return;
    const world = composeThen(n.transform, parentWorld);
    if (typeof n.kind === "object" && "Shape" in n.kind) {
      // Full affine bounds: transform each corner of the shape's local box by the
      // accumulated world transform and take the axis-aligned box, so committed scale
      // (from the PropertiesPanel's W/H fields) shows up and repeated edits don't compound
      // against stale untransformed dims. Handles rotation too, once nodes can have any
      // (corner-transform doesn't care whether a/b/c/d came from scale or rotation).
      const b = shapeBounds(n.kind.Shape);
      const corners = [
        applyAffine(world, b.x, b.y),
        applyAffine(world, b.x + b.w, b.y),
        applyAffine(world, b.x, b.y + b.h),
        applyAffine(world, b.x + b.w, b.y + b.h),
      ];
      const xs = corners.map((c) => c.x);
      const ys = corners.map((c) => c.y);
      const x = Math.min(...xs);
      const y = Math.min(...ys);
      nodes.push({ id: n.id, bounds: { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y } });
    } else {
      for (const child of n.children) walk(child, world);
    }
  };
  walk(doc.root, IDENTITY_AFFINE6);
  return { nodes };
}

export function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<Canvas2DRenderer | null>(null);
  const dragStart = useRef<Pt | null>(null);
  // Ids being dragged, captured at mousedown. Reading `selected` in the move/up handlers
  // would depend on React having flushed the mousedown's setSelected before the next
  // event — true today, but fragile. The ref pins the gesture's selection explicitly.
  const dragIds = useRef<number[]>([]);

  const [doc, setDoc] = useState<DocSnapshot | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const [machines, setMachines] = useState<MachineProfile[]>([]);
  const [tool, setTool] = useState("select");
  const [error, setError] = useState<string | null>(null);

  const scene = useMemo(() => (doc ? buildScene(doc) : { nodes: [] }), [doc]);

  const refresh = useCallback(async () => {
    const json = (await ipc.snapshot()) as string;
    setDoc(JSON.parse(json) as DocSnapshot);
  }, []);

  // ponytail: every command re-fetches the full snapshot instead of applying its returned
  // Delta locally with reconcile() — correct and simple while scenes stay tiny. The canvas
  // drag gesture below uses applyOptimistic for live feedback then also just re-fetches on
  // mouseup; reconcile() stays unused until per-frame delta application is worth the wiring.
  const run = useCallback(
    async (fn: () => Promise<unknown>) => {
      try {
        setError(null);
        await fn();
        await refresh();
        return true;
      } catch (e) {
        // No silent failures: every caught error is surfaced in the status bar.
        setError(String(e));
        return false;
      }
    },
    [refresh],
  );

  useEffect(() => {
    refresh().catch((e) => setError(String(e)));
    ipc
      .listMachines()
      .then((m) => setMachines(m as MachineProfile[]))
      .catch((e) => setError(String(e)));
  }, [refresh]);

  useEffect(() => {
    const ctx = canvasRef.current?.getContext("2d");
    if (ctx) rendererRef.current = new Canvas2DRenderer(ctx);
  }, []);

  useEffect(() => {
    const r = rendererRef.current;
    if (!r) return;
    r.setScene(scene);
    r.setSelection(selected);
    r.setArtboard(doc?.artboard ?? null);
    r.draw();
  }, [scene, selected, doc]);

  // Clears selection only once the delete actually lands, so a failed delete leaves the
  // (still valid) selection in place, and a successful one can't leave stale ids around to
  // error out a later transform.
  const deleteSelected = useCallback(() => {
    run(() => ipc.deleteNodes({ ids: selected })).then((ok) => {
      if (ok) setSelected([]);
    });
  }, [run, selected]);

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
        run(() => (e.shiftKey ? ipc.redo() : ipc.undo()));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected, run, deleteSelected]);

  const canvasPos = (e: MouseEvent<HTMLCanvasElement>): Pt => {
    const rect = e.currentTarget.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  };

  const onCanvasMouseDown = (e: MouseEvent<HTMLCanvasElement>) => {
    const p = canvasPos(e);
    const hit = hitTest(scene, p.x, p.y);
    const next =
      e.shiftKey && hit !== null ? toggleId(selected, hit) : hit === null ? [] : [hit];
    setSelected(next);
    dragIds.current = next;
    // Only start a drag when the hit node is part of the new selection — a
    // shift-click that toggles a node OUT shouldn't begin dragging the rest.
    dragStart.current = hit !== null && next.includes(hit) ? p : null;
  };

  const onCanvasMouseMove = (e: MouseEvent<HTMLCanvasElement>) => {
    const r = rendererRef.current;
    if (!dragStart.current || !r) return;
    const m = dragMatrix(dragStart.current, canvasPos(e));
    r.setScene(applyOptimistic(scene, dragIds.current, m));
    r.setSelection(dragIds.current);
    r.draw();
  };

  // Shared by the canvas's own mouseup and the window-level listener below, so a drag
  // released outside the canvas (mouse left the element before the button came up) still
  // commits instead of leaving the optimistic preview stranded and never saved.
  const finishDrag = useCallback(
    (clientX: number, clientY: number) => {
      const start = dragStart.current;
      dragStart.current = null;
      if (!start || !canvasRef.current) return;
      const rect = canvasRef.current.getBoundingClientRect();
      const m = dragMatrix(start, { x: clientX - rect.left, y: clientY - rect.top });
      if (m[4] === 0 && m[5] === 0) return; // click, not a drag
      const ids = dragIds.current;
      if (ids.length === 0) return;
      run(() => ipc.commitTransform({ ids, m }));
    },
    [run],
  );

  const onCanvasMouseUp = (e: MouseEvent<HTMLCanvasElement>) => finishDrag(e.clientX, e.clientY);

  // Catches mouseup anywhere in the window, not just over the canvas. finishDrag no-ops
  // when dragStart is already null, so this is harmless on the common in-canvas release
  // (which fires first and clears dragStart before this listener runs).
  useEffect(() => {
    const onWindowMouseUp = (e: globalThis.MouseEvent) => finishDrag(e.clientX, e.clientY);
    window.addEventListener("mouseup", onWindowMouseUp);
    return () => window.removeEventListener("mouseup", onWindowMouseUp);
  }, [finishDrag]);

  const root = doc?.root ?? 0;
  const selectedBounds = selected.length === 1 ? (scene.nodes.find((n) => n.id === selected[0])?.bounds ?? null) : null;

  const commitAxis = (axis: "x" | "y", v: number) => {
    if (!selectedBounds) return;
    const m: Matrix = [1, 0, 0, 1, axis === "x" ? v - selectedBounds.x : 0, axis === "y" ? v - selectedBounds.y : 0];
    if (m[4] === 0 && m[5] === 0) return;
    run(() => ipc.commitTransform({ ids: selected, m }));
  };

  // Scale about the bounds origin (x for width, y for height) so the opposite edge stays
  // put: translate(origin) · scale(s) · translate(-origin), i.e. [s,0,0,1, x-s*x, 0] for
  // width and [1,0,0,s, 0, y-s*y] for height.
  const commitScale = (axis: "w" | "h", v: number) => {
    if (!selectedBounds) return;
    const { x, y, w, h } = selectedBounds;
    const size = axis === "w" ? w : h;
    if (size <= 0 || v <= 0) return;
    const s = v / size;
    const m: Matrix = axis === "w" ? [s, 0, 0, 1, x - s * x, 0] : [1, 0, 0, s, 0, y - s * y];
    run(() => ipc.commitTransform({ ids: selected, m }));
  };

  // A successful boolean op removes the source nodes and adds a result node — selecting
  // the removed ids would error the next transform with NotFound, so read the result id
  // straight out of the returned Delta's Add op and select that instead (or clear
  // selection if the shape ever comes back without one).
  const onBooleanOp = useCallback(
    (op: BoolOp) => {
      run(async () => {
        const delta = (await ipc.booleanOp({ ids: selected, op })) as NodeOpJson[];
        const added = delta.find((o): o is Extract<NodeOpJson, { Add: unknown }> => "Add" in o);
        setSelected(added ? [added.Add.node.id] : []);
      });
    },
    [run, selected],
  );

  const onImportFile = (file: File) => {
    run(async () => {
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      const [, skipped] = (await ipc.importSvg({ bytes, parent: root })) as [unknown, string[]];
      if (skipped.length > 0) setError(`Imported with ${skipped.length} element(s) skipped: ${skipped.join(", ")}`);
    });
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
          onSelectMachine={(id) => run(() => ipc.setMachine({ machineId: id }))}
          onSave={() => run(() => ipc.saveProject({ path: PROJECT_PATH }))}
          onReload={() => run(() => ipc.loadProject({ path: PROJECT_PATH }))}
          onUndo={() => run(() => ipc.undo())}
          onRedo={() => run(() => ipc.redo())}
          onImportFile={onImportFile}
        />
      </div>
      <ToolRail
        tool={tool}
        selectionCount={selected.length}
        onSelectTool={setTool}
        onAddRect={() => run(() => ipc.addPrimitive({ parent: root, kind: { Rect: { w: 20, h: 20 } } }))}
        onAddEllipse={() => run(() => ipc.addPrimitive({ parent: root, kind: { Ellipse: { rx: 10, ry: 10 } } }))}
        onAddText={() => run(() => ipc.addText({ parent: root, family: "Arial", sizeMm: 10, text: "Text" }))}
        onBoolean={onBooleanOp}
        onDelete={deleteSelected}
      />
      <canvas
        ref={canvasRef}
        width={800}
        height={600}
        style={{ background: "var(--workspace)" }}
        onMouseDown={onCanvasMouseDown}
        onMouseMove={onCanvasMouseMove}
        onMouseUp={onCanvasMouseUp}
      />
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
        />
      </div>
      <div style={{ gridColumn: "1 / -1" }}>
        <StatusBar machine={doc?.machine ?? null} artboard={doc?.artboard ?? null} error={error} />
      </div>
    </div>
  );
}
