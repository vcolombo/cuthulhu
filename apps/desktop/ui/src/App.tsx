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

function buildScene(doc: DocSnapshot): Scene {
  const nodes: Scene["nodes"] = [];
  const walk = (id: number, ox: number, oy: number) => {
    const n = doc.nodes[id];
    if (!n) return;
    // ponytail: only the translation component (e,f) of each node's transform is
    // accumulated down the tree; rotation/scale (a,b,c,d) are ignored for bounds until
    // full affine bounds are threaded through the Scene type.
    const nx = ox + n.transform[4];
    const ny = oy + n.transform[5];
    if (typeof n.kind === "object" && "Shape" in n.kind) {
      const b = shapeBounds(n.kind.Shape);
      nodes.push({ id: n.id, bounds: { x: nx + b.x, y: ny + b.y, w: b.w, h: b.h } });
    } else {
      for (const child of n.children) walk(child, nx, ny);
    }
  };
  walk(doc.root, 0, 0);
  return { nodes };
}

export function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<Canvas2DRenderer | null>(null);
  const dragStart = useRef<Pt | null>(null);

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
      } catch (e) {
        // No silent failures: every caught error is surfaced in the status bar.
        setError(String(e));
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
    r.draw();
  }, [scene, selected]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      const typing = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA");
      if ((e.key === "Delete" || e.key === "Backspace") && !typing) {
        if (selected.length === 0) return;
        e.preventDefault();
        run(() => ipc.deleteNodes({ ids: selected }));
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z" && !typing) {
        e.preventDefault();
        run(() => (e.shiftKey ? ipc.redo() : ipc.undo()));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected, run]);

  const canvasPos = (e: MouseEvent<HTMLCanvasElement>): Pt => {
    const rect = e.currentTarget.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  };

  const onCanvasMouseDown = (e: MouseEvent<HTMLCanvasElement>) => {
    const p = canvasPos(e);
    const hit = hitTest(scene, p.x, p.y);
    if (e.shiftKey && hit !== null) {
      setSelected((prev) => toggleId(prev, hit));
    } else {
      setSelected(hit === null ? [] : [hit]);
    }
    dragStart.current = hit === null ? null : p;
  };

  const onCanvasMouseMove = (e: MouseEvent<HTMLCanvasElement>) => {
    const r = rendererRef.current;
    if (!dragStart.current || !r) return;
    const m = dragMatrix(dragStart.current, canvasPos(e));
    r.setScene(applyOptimistic(scene, selected, m));
    r.setSelection(selected);
    r.draw();
  };

  const onCanvasMouseUp = (e: MouseEvent<HTMLCanvasElement>) => {
    const start = dragStart.current;
    dragStart.current = null;
    if (!start) return;
    const m = dragMatrix(start, canvasPos(e));
    if (m[4] === 0 && m[5] === 0) return; // click, not a drag
    run(() => ipc.commitTransform({ ids: selected, m }));
  };

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
        onBoolean={(op: BoolOp) => run(() => ipc.booleanOp({ ids: selected, op }))}
        onDelete={() => run(() => ipc.deleteNodes({ ids: selected }))}
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
