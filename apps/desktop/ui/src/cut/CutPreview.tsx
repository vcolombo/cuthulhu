// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useRef } from "react";
import type { Scene } from "../render/hittest";

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

export type PreviewPass = { color: number | null; nodeIds: number[]; enabled: boolean };

type Props = {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  passes: PreviewPass[];
  travel: [number, number, number, number][];
};

/** Cut-plan preview: artboard outline + pass-colored paths + order badges + dashed travel
 *  lines. Same 1px=1mm coordinate mapping as Canvas2DRenderer — no separate viewport
 *  scale/pan, so the scene's world transforms can be drawn directly. */
export function CutPreview({ scene, artboard, passes, travel }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const ctx = canvasRef.current?.getContext("2d");
    if (!ctx) return;
    const canvas = ctx.canvas;
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    const style = getComputedStyle(document.documentElement);
    const border = style.getPropertyValue("--border").trim() || FALLBACK_BORDER;
    const panel = style.getPropertyValue("--panel").trim() || FALLBACK_PANEL;
    const text = style.getPropertyValue("--text").trim() || FALLBACK_TEXT;

    ctx.fillStyle = panel;
    ctx.fillRect(artboard.x, artboard.y, artboard.w, artboard.h);
    ctx.strokeStyle = border;
    ctx.lineWidth = 1;
    ctx.strokeRect(artboard.x, artboard.y, artboard.w, artboard.h);

    passes.forEach((pass, passIndex) => {
      const color = pass.color !== null ? cssColor(pass.color) : text;
      for (const nodeId of pass.nodeIds) {
        const node = scene.nodes.find((n) => n.id === nodeId);
        if (!node) continue;

        ctx.globalAlpha = pass.enabled ? 1 : 0.35;
        ctx.strokeStyle = color;
        ctx.lineWidth = 1;
        if (node.shape && node.world) {
          ctx.save();
          const [a, b, c, d, e, f] = node.world;
          ctx.transform(a, b, c, d, e, f);
          ctx.beginPath();
          if (node.shape.t === "rect") ctx.rect(0, 0, node.shape.w, node.shape.h);
          else if (node.shape.t === "ellipse") ctx.ellipse(node.shape.rx, node.shape.ry, node.shape.rx, node.shape.ry, 0, 0, Math.PI * 2);
          if (node.shape.t === "path") ctx.stroke(new Path2D(node.shape.d));
          else ctx.stroke();
          ctx.restore();
        } else {
          ctx.strokeRect(node.bounds.x, node.bounds.y, node.bounds.w, node.bounds.h);
        }

        // Order badge at the shape's start point (its world-space origin).
        const ox = node.world ? node.world[4] : node.bounds.x;
        const oy = node.world ? node.world[5] : node.bounds.y;
        ctx.globalAlpha = 1;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(ox, oy, 6, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = panel;
        ctx.font = "9px sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(String(passIndex + 1), ox, oy);
      }
    });
    ctx.globalAlpha = 1;

    ctx.strokeStyle = text;
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 3]);
    for (const [x1, y1, x2, y2] of travel) {
      ctx.beginPath();
      ctx.moveTo(x1, y1);
      ctx.lineTo(x2, y2);
      ctx.stroke();
    }
    ctx.setLineDash([]);
  }, [scene, artboard, passes, travel]);

  return <canvas ref={canvasRef} width={400} height={300} style={{ background: "var(--workspace)" }} />;
}
