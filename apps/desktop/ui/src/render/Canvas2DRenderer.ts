// SPDX-License-Identifier: GPL-3.0-or-later
import type { Renderer, NodeId, Overlay } from "./Renderer";
import type { Affine6, Bounds, Scene, ShapeGeom } from "./hittest";
import { apply, compose, IDENTITY, transformBounds } from "./affine";
import { boxCorners, handleWorld, SCALE_HANDLES } from "../interaction/selectionBox";

const FALLBACK_SELECTION = "#012F36";
const FALLBACK_HANDLE = "#012F36";
const FALLBACK_HANDLE_FILL = "#FFFFFF";
const FALLBACK_BORDER = "#CFDADA";
const FALLBACK_PANEL = "#F1F6F6";
const FALLBACK_TEXT = "#012F36";
const FALLBACK_GUIDE = "#B73A80";

// CSS px, whatever the zoom.
const STROKE_PX = 1;
const SELECTED_STROKE_PX = 2;
const HANDLE_PX = 7;
const MARQUEE_ALPHA = 0.12;

export class Canvas2DRenderer implements Renderer {
  private scene: Scene = { nodes: [] };
  private selected = new Set<NodeId>();
  private artboard: Bounds | null = null;
  private view: Affine6 = IDENTITY;
  private dpr = 1;
  private overlay: Overlay = { box: null, marquee: null, guides: [] };
  // ponytail: invalidation only — with the current full-clear+redraw loop this is just
  // a "needs redraw" signal, not a per-node dirty rect. draw() clears it each call.
  private dirty = new Set<NodeId>();
  // Parsed geometry per node, keyed on the geometry's value: a snapshot rebuilds every ShapeGeom
  // object, so object identity would miss after each edit while an equal string still hits.
  private paths = new Map<NodeId, { key: string; path: Path2D }>();

  constructor(private readonly ctx: CanvasRenderingContext2D) {}

  setScene(s: Scene): void {
    this.scene = s;
    // Without this every deleted node's parsed path lives as long as the window.
    const live = new Set(s.nodes.map((n) => n.id));
    for (const id of this.paths.keys()) if (!live.has(id)) this.paths.delete(id);
  }

  setArtboard(b: Bounds | null): void {
    this.artboard = b;
  }

  markDirty(id: NodeId): void {
    this.dirty.add(id);
  }

  setSelection(ids: NodeId[]): void {
    this.selected = new Set(ids);
  }

  resize(cssW: number, cssH: number, dpr: number): void {
    const canvas = this.ctx.canvas;
    canvas.width = Math.max(1, Math.round(cssW * dpr));
    canvas.height = Math.max(1, Math.round(cssH * dpr));
    this.dpr = dpr;
  }

  setView(m: Affine6): void {
    this.view = m;
  }

  setOverlay(o: Overlay): void {
    this.overlay = o;
  }

  draw(): void {
    // ponytail: full clear + redraw every frame instead of patching a dirty region. With the
    // Path2D cache a frame is one matrix multiply and one stroke per node; revisit when the
    // 5 000-path check in MANUAL-CHECKLIST.md drops below 60 fps.
    const { ctx, dpr, view } = this;
    const canvas = ctx.canvas;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    // From here on, units are CSS px: the pixel ratio is applied once so Retina gets real pixels.
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const style = getComputedStyle(document.documentElement);
    // Selection is neutral. `--accent` is a selected-row fill in the chrome, and reading
    // it here would put that teal on the canvas.
    const selection = style.getPropertyValue("--selection").trim() || FALLBACK_SELECTION;
    const handleStroke = style.getPropertyValue("--selection-handle").trim() || FALLBACK_HANDLE;
    // The token table says handles are filled white. `--primary-foreground` is that white
    // in both themes; there is no separate handle-fill token.
    const handleFill = style.getPropertyValue("--primary-foreground").trim() || FALLBACK_HANDLE_FILL;
    const border = style.getPropertyValue("--border").trim() || FALLBACK_BORDER;
    const panel = style.getPropertyValue("--panel").trim() || FALLBACK_PANEL;
    const guide = style.getPropertyValue("--guide").trim() || FALLBACK_GUIDE;
    const text = style.getPropertyValue("--text").trim() || FALLBACK_TEXT;

    // Artboard drawn first so node outlines paint over it, not the other way around.
    if (this.artboard) {
      const a = transformBounds(view, this.artboard);
      ctx.fillStyle = panel;
      ctx.fillRect(a.x, a.y, a.w, a.h);
      ctx.strokeStyle = border;
      ctx.lineWidth = STROKE_PX;
      ctx.strokeRect(a.x, a.y, a.w, a.h);
    }

    for (const node of this.scene.nodes) {
      const selected = this.selected.has(node.id);
      ctx.strokeStyle = selected ? selection : text;
      ctx.lineWidth = selected ? SELECTED_STROKE_PX : STROKE_PX;
      if (node.shape && node.world) {
        // Geometry is carried to screen space before stroking, so the line is in CSS px whatever
        // the zoom or the node's own scale. Stroking under the node's transform scaled it too.
        const onScreen = new Path2D();
        onScreen.addPath(this.localPath(node.id, node.shape), toDOMMatrix(compose(node.world, view)));
        ctx.stroke(onScreen);
      } else {
        // Nodes without geometry (tests, mocks) keep the SP3 bounds outline.
        const b = transformBounds(view, node.bounds);
        ctx.strokeRect(b.x, b.y, b.w, b.h);
      }
    }

    this.drawOverlay(selection, handleStroke, handleFill, guide);
    this.dirty.clear();
  }

  private drawOverlay(selection: string, handleStroke: string, handleFill: string, guide: string): void {
    const { ctx, view } = this;
    const { box, marquee, guides } = this.overlay;
    // Guides first, so the box and handles they line up stay on top of them.
    if (guides.length > 0) {
      ctx.beginPath();
      for (const g of guides) {
        const a = apply(view, g.a);
        const b = apply(view, g.b);
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
      }
      ctx.strokeStyle = guide;
      ctx.lineWidth = STROKE_PX;
      ctx.stroke();
    }
    if (box) {
      const corners = boxCorners(box).map((p) => apply(view, p));
      ctx.beginPath();
      corners.forEach((p, i) => (i === 0 ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y)));
      ctx.closePath();
      ctx.strokeStyle = selection;
      ctx.lineWidth = STROKE_PX;
      ctx.stroke();
      ctx.fillStyle = handleFill;
      ctx.strokeStyle = handleStroke;
      const half = HANDLE_PX / 2;
      for (const h of SCALE_HANDLES) {
        const p = apply(view, handleWorld(box, h));
        ctx.fillRect(p.x - half, p.y - half, HANDLE_PX, HANDLE_PX);
        ctx.strokeRect(p.x - half, p.y - half, HANDLE_PX, HANDLE_PX);
      }
    }
    if (marquee) {
      const m = transformBounds(view, marquee);
      ctx.save();
      ctx.globalAlpha = MARQUEE_ALPHA;
      ctx.fillStyle = selection;
      ctx.fillRect(m.x, m.y, m.w, m.h);
      ctx.restore();
      ctx.save();
      ctx.setLineDash([4, 3]);
      ctx.strokeStyle = selection;
      ctx.lineWidth = STROKE_PX;
      ctx.strokeRect(m.x, m.y, m.w, m.h);
      ctx.restore();
    }
  }

  private localPath(id: NodeId, shape: ShapeGeom): Path2D {
    const key = geomKey(shape);
    const hit = this.paths.get(id);
    if (hit && hit.key === key) return hit.path;
    const path = buildPath(shape);
    this.paths.set(id, { key, path });
    return path;
  }
}

function geomKey(g: ShapeGeom): string {
  if (g.t === "rect") return `r ${g.w} ${g.h}`;
  if (g.t === "ellipse") return `e ${g.rx} ${g.ry}`;
  return `p ${g.d}`;
}

function buildPath(g: ShapeGeom): Path2D {
  if (g.t === "path") return new Path2D(g.d);
  const p = new Path2D();
  if (g.t === "rect") p.rect(0, 0, g.w, g.h);
  // Canonical convention (crates/document/src/commands.rs): local space is centred at (rx, ry).
  else p.ellipse(g.rx, g.ry, g.rx, g.ry, 0, 0, Math.PI * 2);
  return p;
}

function toDOMMatrix([a, b, c, d, e, f]: Affine6): DOMMatrix {
  return new DOMMatrix([a, b, c, d, e, f]);
}
