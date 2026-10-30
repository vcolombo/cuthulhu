// SPDX-License-Identifier: GPL-3.0-or-later
import type { Renderer, NodeId, Overlay } from "./Renderer";
import type { Affine6, Bounds, Scene, ShapeGeom } from "./hittest";
import { apply, compose, IDENTITY, transformBounds } from "./affine";
import { boxCorners, handleWorld, HANDLE_DRAW_PX, SCALE_HANDLES, skewHandlesFor, skewPoint, SKEW_OFFSET_PX } from "../interaction/selectionBox";
import { MM_PER_IN, type LengthUnit } from "../length";
import { vinylDisplay, type VinylDisplay, type VinylTheme } from "./vinyl";

const FALLBACK_SELECTION = "#012F36";
const FALLBACK_HANDLE = "#012F36";
const FALLBACK_HANDLE_FILL = "#FFFFFF";
const FALLBACK_BORDER = "#CFDADA";
const FALLBACK_GUIDE = "#B73A80";
const FALLBACK_MAT = "#D4D4D4";
const FALLBACK_MAT_DARK = "#484848";
const FALLBACK_GRID = "#BABABA";
const FALLBACK_GRID_DARK = "#717171";
const FALLBACK_CANVAS = "#E5EBEB";
const FALLBACK_CANVAS_DARK = "#081416";

// CSS px, whatever the zoom. Vinyl outlines are 2 px so a dark fill that sits close to the mat
// still reads; the selection box stays a hairline around that outline.
const STROKE_PX = 1;
const VINYL_STROKE_PX = 2;
const MARQUEE_ALPHA = 0.1;
const GHOST_OPACITY = 0.03;

type Segment = { x1: number; y1: number; x2: number; y2: number };

export class Canvas2DRenderer implements Renderer {
  private scene: Scene = { nodes: [] };
  private mat: Bounds | null = null;
  private gridUnit: LengthUnit = "in";
  private view: Affine6 = IDENTITY;
  private dpr = 1;
  private overlay: Overlay = { box: null, marquee: null, crossing: false, lasso: null, guides: [] };
  // ponytail: invalidation only — with the current full-clear+redraw loop this is just
  // a "needs redraw" signal, not a per-node dirty rect. draw() clears it each call.
  private dirty = new Set<NodeId>();
  // Parsed geometry per node, keyed on the geometry's value: a snapshot rebuilds every ShapeGeom
  // object, so object identity would miss after each edit while an equal string still hits.
  private paths = new Map<NodeId, { key: string; path: Path2D }>();
  // OKLCH conversion is the expensive part of a frame. A 5 000-path redraw must not redo it.
  private displays = new Map<string, VinylDisplay>();

  constructor(private readonly ctx: CanvasRenderingContext2D) {}

  setScene(s: Scene): void {
    this.scene = s;
    // Without this every deleted node's parsed path lives as long as the window.
    const live = new Set(s.nodes.map((n) => n.id));
    for (const id of this.paths.keys()) if (!live.has(id)) this.paths.delete(id);
  }

  setMat(b: Bounds | null): void {
    this.mat = b;
  }

  setGridUnit(unit: LengthUnit): void {
    this.gridUnit = unit;
  }

  markDirty(id: NodeId): void {
    this.dirty.add(id);
  }

  setSelection(ids: NodeId[]): void {
    // The vinyl outline no longer changes when a shape is selected; the overlay box does that.
    // The ids still mark the frame dirty so a selection-only change is not a silent no-op for a
    // future dirty-rect redraw.
    for (const id of ids) this.dirty.add(id);
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
    // Path2D cache a frame is one matrix multiply and one fill plus one stroke per node; revisit
    // when the 5 000-path check in MANUAL-CHECKLIST.md drops below 60 fps.
    const { ctx, dpr, view } = this;
    const canvas = ctx.canvas;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    // From here on, units are CSS px: the pixel ratio is applied once so Retina gets real pixels.
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const style = getComputedStyle(document.documentElement);
    const dark = document.documentElement.classList.contains("dark");
    const theme: VinylTheme = dark ? "dark" : "light";
    // Selection is neutral. `--accent` is a selected-row fill in the chrome, and reading
    // it here would put that teal on the canvas.
    const selection = style.getPropertyValue("--selection").trim() || FALLBACK_SELECTION;
    const handleStroke = style.getPropertyValue("--selection-handle").trim() || FALLBACK_HANDLE;
    // The token table says handles are filled white. `--primary-foreground` is that white
    // in both themes; there is no separate handle-fill token.
    const handleFill = style.getPropertyValue("--primary-foreground").trim() || FALLBACK_HANDLE_FILL;
    const border = style.getPropertyValue("--border").trim() || FALLBACK_BORDER;
    const guide = style.getPropertyValue("--guide").trim() || FALLBACK_GUIDE;
    const matFill = style.getPropertyValue("--mat").trim() || (dark ? FALLBACK_MAT_DARK : FALLBACK_MAT);
    const matGrid = style.getPropertyValue("--mat-grid").trim() || (dark ? FALLBACK_GRID_DARK : FALLBACK_GRID);
    const surround = style.getPropertyValue("--canvas").trim() || (dark ? FALLBACK_CANVAS_DARK : FALLBACK_CANVAS);

    const cssW = canvas.width / dpr;
    const cssH = canvas.height / dpr;
    ctx.fillStyle = surround;
    ctx.fillRect(0, 0, cssW, cssH);

    if (this.mat) {
      const a = transformBounds(view, this.mat);
      ctx.fillStyle = matFill;
      ctx.fillRect(a.x, a.y, a.w, a.h);
      drawGrid(ctx, view, this.mat, this.gridUnit, matGrid, cssW, cssH);
      ctx.strokeStyle = border;
      ctx.lineWidth = STROKE_PX;
      ctx.strokeRect(a.x, a.y, a.w, a.h);
    }

    // Fills first, in document order, then every outline. A later fill covers an earlier shape,
    // and the second pass puts that shape's edge back on top, which is what keeps a gold circle
    // inside a red one visible.
    const drawn = this.scene.nodes.map((node) => ({
      path: this.screenPath(node),
      display: this.displayFor(node.vinyl, theme, node.cut === false),
      dashed: node.vinyl == null || node.cut === false,
    }));

    for (const node of drawn) {
      if (!node.display) continue;
      ctx.save();
      ctx.globalAlpha = node.display.fillOpacity;
      ctx.fillStyle = node.display.fill;
      ctx.fill(node.path);
      ctx.restore();
    }

    ctx.lineWidth = VINYL_STROKE_PX;
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    for (const node of drawn) {
      ctx.setLineDash(node.dashed ? [4, 3] : []);
      ctx.strokeStyle = node.display ? node.display.stroke : guide;
      ctx.stroke(node.path);
    }
    ctx.setLineDash([]);
    ctx.lineJoin = "miter";
    ctx.lineCap = "butt";

    this.drawOverlay(selection, handleStroke, handleFill, guide);
    this.dirty.clear();
  }

  private displayFor(vinyl: number | null | undefined, theme: VinylTheme, ghost: boolean): VinylDisplay | null {
    if (vinyl == null) return null;
    const key = `${vinyl}:${theme}:${ghost ? 1 : 0}`;
    const hit = this.displays.get(key);
    if (hit) return hit;
    const next = vinylDisplay(vinyl, theme, ghost ? GHOST_OPACITY : undefined);
    this.displays.set(key, next);
    return next;
  }

  private screenPath(node: Scene["nodes"][number]): Path2D {
    const onScreen = new Path2D();
    if (node.shape && node.world) {
      onScreen.addPath(this.localPath(node.id, node.shape), toDOMMatrix(compose(node.world, this.view)));
    } else {
      const b = transformBounds(this.view, node.bounds);
      onScreen.rect(b.x, b.y, b.w, b.h);
    }
    return onScreen;
  }

  private drawOverlay(selection: string, handleStroke: string, handleFill: string, guide: string): void {
    const { ctx, view } = this;
    const { box, marquee, crossing, lasso, guides } = this.overlay;
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
    // The band and the loop go under the box so a handle sitting on the band can still be grabbed.
    if (marquee) {
      const m = transformBounds(view, marquee);
      ctx.save();
      ctx.globalAlpha = MARQUEE_ALPHA;
      ctx.fillStyle = selection;
      ctx.fillRect(m.x, m.y, m.w, m.h);
      ctx.restore();
      ctx.save();
      // Dashed: the band selects what it crosses. Solid: only what it contains.
      if (crossing) ctx.setLineDash([4, 3]);
      ctx.strokeStyle = selection;
      ctx.lineWidth = STROKE_PX;
      ctx.strokeRect(m.x, m.y, m.w, m.h);
      ctx.restore();
    }
    if (lasso && lasso.length > 1) {
      ctx.beginPath();
      lasso.forEach((p, i) => {
        const s = apply(view, p);
        if (i === 0) ctx.moveTo(s.x, s.y);
        else ctx.lineTo(s.x, s.y);
      });
      ctx.closePath();
      ctx.save();
      ctx.globalAlpha = MARQUEE_ALPHA;
      ctx.fillStyle = selection;
      ctx.fill();
      ctx.restore();
      ctx.setLineDash([4, 3]);
      ctx.strokeStyle = selection;
      ctx.lineWidth = STROKE_PX;
      ctx.stroke();
      ctx.setLineDash([]);
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
      const half = HANDLE_DRAW_PX / 2;
      const scale = view[0];
      if (scale > 0) {
        for (const h of skewHandlesFor(box)) {
          const p = apply(view, skewPoint(box, h, SKEW_OFFSET_PX / scale));
          ctx.beginPath();
          ctx.moveTo(p.x, p.y - half);
          ctx.lineTo(p.x + half, p.y);
          ctx.lineTo(p.x, p.y + half);
          ctx.lineTo(p.x - half, p.y);
          ctx.closePath();
          ctx.fill();
          ctx.stroke();
        }
      }
      for (const h of SCALE_HANDLES) {
        const p = apply(view, handleWorld(box, h));
        ctx.fillRect(p.x - half, p.y - half, HANDLE_DRAW_PX, HANDLE_DRAW_PX);
        ctx.strokeRect(p.x - half, p.y - half, HANDLE_DRAW_PX, HANDLE_DRAW_PX);
      }
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

function drawGrid(
  ctx: CanvasRenderingContext2D,
  view: Affine6,
  mat: Bounds,
  unit: LengthUnit,
  color: string,
  cssW: number,
  cssH: number,
): void {
  const scale = view[0];
  const tx = view[4];
  const ty = view[5];
  if (!(scale > 0)) return;
  // Inches: a line every 1/4 in, darker every inch. Millimetres: every 5 mm, darker every centimetre.
  const minor = unit === "in" ? MM_PER_IN / 4 : 5;
  const major = unit === "in" ? MM_PER_IN : 10;
  const left = Math.max(mat.x, (0 - tx) / scale);
  const right = Math.min(mat.x + mat.w, (cssW - tx) / scale);
  const top = Math.max(mat.y, (0 - ty) / scale);
  const bottom = Math.min(mat.y + mat.h, (cssH - ty) / scale);
  if (right <= left || bottom <= top) return;
  // Each axis on its own: a long roll is tall and a wide machine is not, and one step
  // for both would coarsen the short side.
  const xStep = gridStep(minor, major, scale, right - left);
  const yStep = gridStep(minor, major, scale, bottom - top);

  const minorSegs: Segment[] = [];
  const majorSegs: Segment[] = [];
  const sy1 = top * scale + ty;
  const sy2 = bottom * scale + ty;
  for (const x of steps(mat.x, xStep, left, right)) {
    const sx = x * scale + tx;
    (onMajor(x, mat.x, major) ? majorSegs : minorSegs).push({ x1: sx, y1: sy1, x2: sx, y2: sy2 });
  }
  const sx1 = left * scale + tx;
  const sx2 = right * scale + tx;
  for (const y of steps(mat.y, yStep, top, bottom)) {
    const sy = y * scale + ty;
    (onMajor(y, mat.y, major) ? majorSegs : minorSegs).push({ x1: sx1, y1: sy, x2: sx2, y2: sy });
  }
  strokeSegments(ctx, color, 0.5, minorSegs);
  strokeSegments(ctx, color, 1, majorSegs);
}

/**
 * Minor lines while they stay at least 4 CSS px apart. Otherwise the major step, and a
 * multiple of that when the visible span would still pass the 500-coordinate cap in
 * `steps`. A long roll at the zoom floor is denser than an inch even on the major step,
 * and the cap used to stop drawing and leave the rest of the mat blank.
 */
export function gridStep(minor: number, major: number, scale: number, span: number): number {
  const base = minor * scale >= 4 ? minor : major;
  if (span <= base * 499) return base;
  return major * Math.ceil(span / (499 * major));
}

function steps(origin: number, step: number, min: number, max: number): number[] {
  const out: number[] = [];
  const first = origin + Math.ceil((min - origin) / step - 1e-9) * step;
  for (let v = first; v <= max + step * 1e-6; v += step) {
    out.push(v);
    if (out.length > 500) break;
  }
  return out;
}

function onMajor(value: number, origin: number, major: number): boolean {
  const k = (value - origin) / major;
  return Math.abs(k - Math.round(k)) < 1e-4;
}

function strokeSegments(ctx: CanvasRenderingContext2D, color: string, alpha: number, segments: Segment[]): void {
  if (segments.length === 0) return;
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.strokeStyle = color;
  ctx.lineWidth = STROKE_PX;
  ctx.beginPath();
  for (const s of segments) {
    ctx.moveTo(s.x1, s.y1);
    ctx.lineTo(s.x2, s.y2);
  }
  ctx.stroke();
  ctx.restore();
}
