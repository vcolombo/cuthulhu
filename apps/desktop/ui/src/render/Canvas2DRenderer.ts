// SPDX-License-Identifier: GPL-3.0-or-later
import type { Renderer, NodeId } from "./Renderer";
import type { Bounds, Scene } from "./hittest";

const FALLBACK_ACCENT = "#22D3EE";
const FALLBACK_BORDER = "#2E2E34";
const FALLBACK_PANEL = "#1F1F23";
const FALLBACK_TEXT = "#E7E7EA";

export class Canvas2DRenderer implements Renderer {
  private scene: Scene = { nodes: [] };
  private selected = new Set<NodeId>();
  private artboard: Bounds | null = null;
  // ponytail: invalidation only — with the current full-clear+redraw loop this is just
  // a "needs redraw" signal, not a per-node dirty rect. draw() clears it each call.
  private dirty = new Set<NodeId>();

  constructor(private readonly ctx: CanvasRenderingContext2D) {}

  setScene(s: Scene): void {
    this.scene = s;
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

  draw(): void {
    // ponytail: full clear + redraw every frame instead of tracking/patching just the
    // dirty region — scenes are tiny during scaffolding. Revisit once real path
    // rendering and larger scenes make a full repaint measurably expensive.
    const { ctx } = this;
    const canvas = ctx.canvas;
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    const style = getComputedStyle(document.documentElement);
    const accent = style.getPropertyValue("--accent").trim() || FALLBACK_ACCENT;
    const border = style.getPropertyValue("--border").trim() || FALLBACK_BORDER;
    const panel = style.getPropertyValue("--panel").trim() || FALLBACK_PANEL;
    const text = style.getPropertyValue("--text").trim() || FALLBACK_TEXT;

    // Artboard drawn first so node outlines paint over it, not the other way around.
    if (this.artboard) {
      const { x, y, w, h } = this.artboard;
      ctx.fillStyle = panel;
      ctx.fillRect(x, y, w, h);
      ctx.strokeStyle = border;
      ctx.lineWidth = 1;
      ctx.strokeRect(x, y, w, h);
    }

    for (const node of this.scene.nodes) {
      const selected = this.selected.has(node.id);
      const stroke = selected ? accent : text;
      const lineWidth = selected ? 2 : 1;
      if (node.shape && node.world) {
        ctx.save();
        const [a, b, c, d, e, f] = node.world;
        ctx.transform(a, b, c, d, e, f);
        ctx.beginPath();
        if (node.shape.t === "rect") {
          ctx.rect(0, 0, node.shape.w, node.shape.h);
        } else if (node.shape.t === "ellipse") {
          ctx.ellipse(node.shape.rx, node.shape.ry, node.shape.rx, node.shape.ry, 0, 0, Math.PI * 2);
        }
        ctx.strokeStyle = stroke;
        // ponytail: lineWidth is in local units, so a scaled node gets a scaled
        // stroke; screen-constant strokes need lineWidth / scale once zoom lands.
        ctx.lineWidth = lineWidth;
        if (node.shape.t === "path") {
          ctx.stroke(new Path2D(node.shape.d));
        } else {
          ctx.stroke();
        }
        ctx.restore();
      } else {
        // Nodes without geometry (tests, mocks) keep the SP3 bounds outline.
        ctx.strokeStyle = stroke;
        ctx.lineWidth = lineWidth;
        ctx.strokeRect(node.bounds.x, node.bounds.y, node.bounds.w, node.bounds.h);
      }
    }

    this.dirty.clear();
  }
}
