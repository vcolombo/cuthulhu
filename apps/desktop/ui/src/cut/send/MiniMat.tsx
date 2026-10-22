// SPDX-License-Identifier: GPL-3.0-or-later
import type { PassKey } from "../../ipc";
import type { Scene } from "../../render/hittest";
import { vinylDisplay } from "../../render/vinyl";
import type { ResolvedTheme } from "../../theme";
import { parsePassKey } from "../viewmodel";
import type { PassRow } from "./useCutPlan";

/** The shapes of one pass, drawn on the same mat as the rest of the plan so a row can be
 *  checked against the sheet without leaving step 2. Dimmed shapes are the other passes. */
export function MiniMat({
  scene,
  artboard,
  passes,
  highlight,
  theme,
}: {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  passes: PassRow[];
  highlight: number | null;
  theme: ResolvedTheme;
}) {
  const byId = new Map(scene.nodes.map((node) => [node.id, node]));
  const highlighted = highlight === null ? null : passes[highlight] ?? null;
  const boxes = passes.flatMap((pass) => pass.nodeIds.map((id) => byId.get(id)?.bounds)).filter((box) => box !== undefined);
  const box = boxes.length === 0
    ? artboard
    : pad(union(boxes), artboard);
  return (
    <svg
      viewBox={`${box.x} ${box.y} ${box.w} ${box.h}`}
      className="h-28 w-auto shrink-0"
      role="img"
      aria-label="Shapes in this pass"
    >
      <rect x={box.x} y={box.y} width={box.w} height={box.h} fill="var(--mat)" rx={box.w * 0.02} />
      {passes.map((pass, index) => pass.nodeIds.map((id) => {
        const node = byId.get(id);
        if (!node) return null;
        const on = highlighted !== null && highlighted.nodeIds.includes(id) && index === highlight;
        const fromPass = passColour(pass.key, theme);
        const swatch = node.vinyl != null ? vinylDisplay(node.vinyl, theme).swatch : fromPass ?? "var(--muted-foreground)";
        return (
          <rect
            key={`${pass.key}:${id}`}
            x={node.bounds.x}
            y={node.bounds.y}
            width={Math.max(node.bounds.w, 0.2)}
            height={Math.max(node.bounds.h, 0.2)}
            fill={on ? swatch : "var(--muted-foreground)"}
            opacity={on ? 0.9 : 0.18}
          />
        );
      }))}
    </svg>
  );
}

function passColour(key: PassKey, theme: ResolvedTheme): string | null {
  const parsed = parsePassKey(key);
  return parsed.kind === "color" && parsed.color !== null ? vinylDisplay(parsed.color, theme).swatch : null;
}

function union(boxes: { x: number; y: number; w: number; h: number }[]) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const box of boxes) {
    x0 = Math.min(x0, box.x);
    y0 = Math.min(y0, box.y);
    x1 = Math.max(x1, box.x + box.w);
    y1 = Math.max(y1, box.y + box.h);
  }
  return { x: x0, y: y0, w: Math.max(x1 - x0, 1), h: Math.max(y1 - y0, 1) };
}

/** A margin around the shapes, without wandering off into an empty roll. */
function pad(
  box: { x: number; y: number; w: number; h: number },
  artboard: { x: number; y: number; w: number; h: number },
) {
  const margin = Math.max(box.w, box.h) * 0.2;
  const x = Math.max(artboard.x, box.x - margin);
  const y = Math.max(artboard.y, box.y - margin);
  const x1 = Math.min(artboard.x + artboard.w, box.x + box.w + margin);
  const y1 = Math.min(artboard.y + artboard.h, box.y + box.h + margin);
  return { x, y, w: Math.max(x1 - x, 1), h: Math.max(y1 - y, 1) };
}
