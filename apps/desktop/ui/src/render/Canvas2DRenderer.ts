// SPDX-License-Identifier: GPL-3.0-or-later
import type { Renderer, NodeId } from "./Renderer";
import type { Scene } from "./hittest";

const FALLBACK_ACCENT = "#22D3EE";
const FALLBACK_BORDER = "#2E2E34";

export class Canvas2DRenderer implements Renderer {
  private scene: Scene = { nodes: [] };
  // ponytail: markDirty ids double as the "selected" set for now — there's no
  // selection channel yet (comes with the panels/selection work in later tasks).
  private dirty = new Set<NodeId>();

  constructor(private readonly ctx: CanvasRenderingContext2D) {}

  setScene(s: Scene): void {
    this.scene = s;
  }

  markDirty(id: NodeId): void {
    this.dirty.add(id);
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

    for (const node of this.scene.nodes) {
      const selected = this.dirty.has(node.id);
      ctx.strokeStyle = selected ? accent : border;
      ctx.lineWidth = selected ? 2 : 1;
      ctx.strokeRect(node.bounds.x, node.bounds.y, node.bounds.w, node.bounds.h);
    }
  }
}
