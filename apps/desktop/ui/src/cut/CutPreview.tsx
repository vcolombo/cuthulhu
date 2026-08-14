// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useRef } from "react";
import type { Scene } from "../render/hittest";
import { clippedEdges, contentBounds, fitViewport } from "./viewmodel";

const FALLBACK_BORDER = "#2E2E34";
const FALLBACK_PANEL = "#1F1F23";
const FALLBACK_TEXT = "#E7E7EA";

/** Converts a packed 0xRRGGBBAA stroke color (see document::Style) to a CSS rgba() string. */
export function cssColor(rgba: number): string {
  const r = (rgba >>> 24) & 0xff;
  const g = (rgba >>> 16) & 0xff;
  const b = (rgba >>> 8) & 0xff;
  const a = (rgba & 0xff) / 255;
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

export type PreviewPass = {
  color: number | null;
  nodeIds: number[];
  /** Each shape's first world-space point from the plan, parallel to nodeIds. */
  starts: ([number, number] | null)[];
  enabled: boolean;
};

type Props = {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  passes: PreviewPass[];
  travel: [number, number, number, number][];
};

/** Cut-plan preview: artboard outline + pass-colored paths + order badges + dashed travel
 *  lines. The viewport fits the planned content (falling back to the artboard when the
 *  plan is empty) — a fixed 1px=1mm mapping left a typical small document a few dozen
 *  pixels in the corner of a 330×3000mm artboard. Badges and dashes are screen-space so
 *  they stay legible at any fitted scale. */
export function CutPreview({ scene, artboard, passes, travel }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const ctx = canvasRef.current?.getContext("2d");
    if (!ctx) return;
    const canvas = ctx.canvas;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    const style = getComputedStyle(document.documentElement);
    const border = style.getPropertyValue("--border").trim() || FALLBACK_BORDER;
    const panel = style.getPropertyValue("--panel").trim() || FALLBACK_PANEL;
    const text = style.getPropertyValue("--text").trim() || FALLBACK_TEXT;

    const nodesById = new Map(scene.nodes.map((n) => [n.id, n]));
    const drawn = passes.flatMap((p) => p.nodeIds)
      .map((id) => nodesById.get(id)?.bounds)
      .filter((b): b is NonNullable<typeof b> => b !== undefined);
    const size = { w: canvas.width, h: canvas.height };
    const vp = fitViewport(contentBounds(drawn, travel), artboard, size, 16);

    ctx.setTransform(vp.scale, 0, 0, vp.scale, vp.tx, vp.ty);
    ctx.fillStyle = panel;
    ctx.fillRect(artboard.x, artboard.y, artboard.w, artboard.h);
    ctx.strokeStyle = border;
    ctx.lineWidth = 1 / vp.scale;
    ctx.strokeRect(artboard.x, artboard.y, artboard.w, artboard.h);

    // Shapes are mapped to screen space as geometry (addPath with the composed
    // viewport×world matrix) and stroked under the identity transform, so the
    // stroke is a uniform 1px regardless of node scaling — stroking under a
    // non-uniform world transform renders a 0.1x-scaled node's edges at 0.1px,
    // which is reachable today through the PropertiesPanel's W/H fields.
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    const vpM = new DOMMatrix([vp.scale, 0, 0, vp.scale, vp.tx, vp.ty]);
    passes.forEach((pass) => {
      const color = pass.color !== null ? cssColor(pass.color) : text;
      for (const nodeId of pass.nodeIds) {
        const node = nodesById.get(nodeId);
        if (!node) continue;

        ctx.globalAlpha = pass.enabled ? 1 : 0.35;
        ctx.strokeStyle = color;
        ctx.lineWidth = 1;
        const screen = new Path2D();
        if (node.shape && node.world) {
          const [a, b, c, d, e, f] = node.world;
          let local: Path2D;
          if (node.shape.t === "path") local = new Path2D(node.shape.d);
          else {
            local = new Path2D();
            if (node.shape.t === "rect") local.rect(0, 0, node.shape.w, node.shape.h);
            else local.ellipse(node.shape.rx, node.shape.ry, node.shape.rx, node.shape.ry, 0, 0, Math.PI * 2);
          }
          screen.addPath(local, vpM.multiply(new DOMMatrix([a, b, c, d, e, f])));
        } else {
          const local = new Path2D();
          local.rect(node.bounds.x, node.bounds.y, node.bounds.w, node.bounds.h);
          screen.addPath(local, vpM);
        }
        ctx.stroke(screen);
      }
    });
    ctx.globalAlpha = 1;

    ctx.strokeStyle = text;
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 3]);
    for (const [x1, y1, x2, y2] of travel) {
      ctx.beginPath();
      ctx.moveTo(x1 * vp.scale + vp.tx, y1 * vp.scale + vp.ty);
      ctx.lineTo(x2 * vp.scale + vp.tx, y2 * vp.scale + vp.ty);
      ctx.stroke();
    }
    ctx.setLineDash([]);

    // Badges and clip markers draw in screen space — a badge that scaled with the
    // fit is exactly the illegibility this viewport exists to remove.
    passes.forEach((pass, passIndex) => {
      const color = pass.color !== null ? cssColor(pass.color) : text;
      pass.nodeIds.forEach((nodeId, shapeIndex) => {
        const node = nodesById.get(nodeId);
        if (!node) return;
        // Order badge at the shape's first planned point — where the blade actually
        // lands (PR #141 wanted this; the plan now carries it). A shape whose outline
        // flattened to nothing falls back to the world-bounds corner, which is inside
        // the fitted region by construction.
        const start = pass.starts[shapeIndex];
        const bx = (start ? start[0] : node.bounds.x) * vp.scale + vp.tx;
        const by = (start ? start[1] : node.bounds.y) * vp.scale + vp.ty;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(bx, by, 6, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = panel;
        ctx.font = "9px sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(String(passIndex + 1), bx, by);
      });
    });

    // Where the fitted view cuts the artboard off, a dashed line along the canvas
    // edge says "the sheet continues past here" — otherwise a missing border edge
    // reads as the edge of the material.
    const clipped = clippedEdges(vp, artboard, size);
    ctx.strokeStyle = border;
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 4]);
    const edge = (x1: number, y1: number, x2: number, y2: number) => {
      ctx.beginPath();
      ctx.moveTo(x1, y1);
      ctx.lineTo(x2, y2);
      ctx.stroke();
    };
    if (clipped.left) edge(0.5, 0, 0.5, size.h);
    if (clipped.right) edge(size.w - 0.5, 0, size.w - 0.5, size.h);
    if (clipped.top) edge(0, 0.5, size.w, 0.5);
    if (clipped.bottom) edge(0, size.h - 0.5, size.w, size.h - 0.5);
    ctx.setLineDash([]);
  }, [scene, artboard, passes, travel]);

  return <canvas ref={canvasRef} width={400} height={300} style={{ background: "var(--workspace)" }} />;
}
