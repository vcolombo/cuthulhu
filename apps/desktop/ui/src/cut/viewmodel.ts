// SPDX-License-Identifier: GPL-3.0-or-later
// View model types (UI representation)
export type PassVm = {
  color: number | null;
  shapeCount: number;
  enabled: boolean;
  presetId: string | null;
  speed: number | null;
  force: number | null;
  repeatCount: number | null;
};

export type Caps = {
  supportsSpeed: boolean;
  supportsForce: boolean;
  needsOperatorPassConfirm: boolean;
};

// Wire types (match Rust ConfiguredPassDto and CutRequest)
export type ConfiguredPassDto = {
  color: number | null;
  enabled: boolean;
  preset_id: string | null;
  speed: number | null;
  force: number | null;
  repeat_count: number | null;
};

export type CutRequest = {
  device_instance_id: string;
  doc_revision: string;
  passes: ConfiguredPassDto[];
};

// Preset type (mirrors cutplan::MaterialPreset)
export type Preset = {
  id: string;
  name: string;
  machine_id: string;
  settings: {
    speed: number | null;
    force: number | null;
    repeat_count: number;
  };
  builtin: boolean;
};

export type Bounds = { x: number; y: number; w: number; h: number };

/** World-mm → canvas-px mapping for the cut preview: screen = world * scale + t. */
export type Viewport = { scale: number; tx: number; ty: number };

/**
 * Union of the geometry the preview will actually draw: pass-member shape bounds
 * plus travel endpoints. Travel counts because the park/origin moves routinely
 * leave the content box, and a fit that crops them defeats the dashed lines'
 * purpose. Returns null when there is nothing to draw (the empty-plan case).
 */
export function contentBounds(
  shapeBounds: Bounds[],
  travel: [number, number, number, number][]
): Bounds | null {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const b of shapeBounds) {
    minX = Math.min(minX, b.x); minY = Math.min(minY, b.y);
    maxX = Math.max(maxX, b.x + b.w); maxY = Math.max(maxY, b.y + b.h);
  }
  for (const [x1, y1, x2, y2] of travel) {
    minX = Math.min(minX, x1, x2); minY = Math.min(minY, y1, y2);
    maxX = Math.max(maxX, x1, x2); maxY = Math.max(maxY, y1, y2);
  }
  if (minX === Infinity) return null;
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
}

/**
 * Fit `content` (falling back to the artboard for an empty plan) into the canvas
 * with a uniform scale and a symmetric pixel margin, centered on both axes.
 * Degenerate extents are widened to 1mm so a single dot fits at a finite scale
 * instead of an Infinity that blanks the canvas.
 */
export function fitViewport(
  content: Bounds | null,
  artboard: Bounds,
  canvas: { w: number; h: number },
  marginPx: number
): Viewport {
  const target = content ?? artboard;
  const minMm = 1;
  const w = Math.max(target.w, minMm);
  const h = Math.max(target.h, minMm);
  // Widening keeps the original box centered, so re-center the target too.
  const cx = target.x + target.w / 2;
  const cy = target.y + target.h / 2;
  // A margin that swallows the canvas would flip the scale negative and invert the
  // render; clamp the drawable area so scale stays positive and finite for any input.
  const availW = Math.max(canvas.w - 2 * marginPx, 1);
  const availH = Math.max(canvas.h - 2 * marginPx, 1);
  const scale = Math.min(availW / w, availH / h);
  return { scale, tx: canvas.w / 2 - cx * scale, ty: canvas.h / 2 - cy * scale };
}

/**
 * Which canvas edges cut the artboard off. The artboard rect itself is drawn
 * clipped for free by the canvas; these flags drive the explicit "it continues
 * past here" treatment so a missing border edge reads as clipped, not absent.
 */
export function clippedEdges(
  vp: Viewport,
  artboard: Bounds,
  canvas: { w: number; h: number }
): { left: boolean; right: boolean; top: boolean; bottom: boolean } {
  const x1 = artboard.x * vp.scale + vp.tx;
  const y1 = artboard.y * vp.scale + vp.ty;
  const x2 = (artboard.x + artboard.w) * vp.scale + vp.tx;
  const y2 = (artboard.y + artboard.h) * vp.scale + vp.ty;
  return { left: x1 < 0, right: x2 > canvas.w, top: y1 < 0, bottom: y2 > canvas.h };
}

/**
 * Reorder a pass within the list by swapping it with an adjacent element.
 * Clamps at the start and end bounds.
 *
 * @param passes The list of passes
 * @param index The index of the pass to move
 * @param dir Direction: -1 (up) or 1 (down)
 * @returns A new array with the reordered passes
 */
export function reorderPass<T>(
  passes: T[],
  index: number,
  dir: -1 | 1
): T[] {
  // Clamp: if at boundary in the direction of movement, return unchanged
  if (dir === -1 && index === 0) return passes;
  if (dir === 1 && index === passes.length - 1) return passes;

  const newIndex = index + dir;
  const result = [...passes];
  [result[index], result[newIndex]] = [result[newIndex], result[index]];

  return result;
}

/**
 * Compute effective settings for a pass, accounting for overrides and presets.
 * Priority: pass override > preset > default (repeatCount defaults to 1)
 *
 * @param p The pass
 * @param presets Available presets
 * @returns Effective settings with resolved speed, force, and repeatCount
 */
export function effectiveSettings(
  p: PassVm,
  presets: Preset[]
): {
  speed: number | null;
  force: number | null;
  repeatCount: number;
} {
  // Find matching preset
  const preset = p.presetId ? presets.find((pr) => pr.id === p.presetId) : null;

  // Speed: pass override > preset > null
  const speed =
    p.speed !== null ? p.speed : preset?.settings.speed ?? null;

  // Force: pass override > preset > null
  const force =
    p.force !== null ? p.force : preset?.settings.force ?? null;

  // RepeatCount: pass override > preset > 1
  const repeatCount =
    p.repeatCount !== null
      ? p.repeatCount
      : preset?.settings.repeat_count ?? 1;

  return { speed, force, repeatCount };
}

/**
 * Determine if a field is disabled based on device capabilities.
 *
 * @param field The field name ("speed" or "force")
 * @param caps Device capabilities
 * @returns true if the field should be disabled
 */
export function fieldDisabled(
  field: "speed" | "force",
  caps: Caps
): boolean {
  if (field === "speed") return !caps.supportsSpeed;
  return !caps.supportsForce;
}

/**
 * Convert PassVm[] to CutRequest for transmission to Rust backend.
 * Maps camelCase PassVm to snake_case ConfiguredPassDto fields.
 *
 * @param deviceInstanceId Device instance ID
 * @param docRevision Document revision
 * @param passes Array of passes
 * @returns CutRequest ready to send to backend
 */
export function toCutRequest(
  deviceInstanceId: string,
  docRevision: string,
  passes: PassVm[]
): CutRequest {
  return {
    device_instance_id: deviceInstanceId,
    doc_revision: docRevision,
    passes: passes.map((p) => ({
      color: p.color,
      enabled: p.enabled,
      preset_id: p.presetId,
      speed: p.speed,
      force: p.force,
      repeat_count: p.repeatCount,
    })),
  };
}
