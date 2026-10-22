// SPDX-License-Identifier: GPL-3.0-or-later
import type { Affine6, Bounds } from "../render/hittest";
import type { Pt } from "../render/affine";

/** screen = world · scale + (tx, ty). World is document mm; screen is CSS px from the canvas's
 *  top-left. Uniform scale only: nobody wants a squashed view of a cut. */
export type View = { scale: number; tx: number; ty: number };
export type Size = { w: number; h: number };

/** "100%" is CSS reference size: 96 CSS px to the CSS inch. That is physical size only when the
 *  display's density and the OS's scaling match the reference pixel; on most screens it is close,
 *  not exact (Copilot on #298). The document is in millimetres.
 *  ponytail: no per-monitor calibration. Ceiling: ⌘1 is "about real size", not ruler-exact.
 *  Upgrade: a one-time "hold a card to the screen" calibration stored per display. */
export const CSS_PX_PER_MM = 96 / 25.4;
export const MAX_SCALE = 64 * CSS_PX_PER_MM; // 6400%
/** Air around a fitted mat, inside whatever the caller insets (rulers, the zoom bar). */
export const FIT_MARGIN_PX = 32;
/** Air around the artwork for Fit content. Wider than the mat fit on purpose. */
export const CONTENT_FIT_MARGIN_PX = 80;
export const ZOOM_STEP = 1.25;

/** CSS px the fit must leave clear: rulers on the left and top, nothing required on the other edges. */
export type Inset = { left: number; top: number; right: number; bottom: number };

const ZERO_INSET: Inset = { left: 0, top: 0, right: 0, bottom: 0 };
/** Pinch reports a few px per event and a mouse notch about 100. An exponential keeps both
 *  proportional, and zooming in then out by the same delta lands exactly where it started. */
export const WHEEL_ZOOM_RATE = 0.01;

export const IDENTITY_VIEW: View = { scale: 1, tx: 0, ty: 0 };

export function fitView(target: Bounds, size: Size, margin = FIT_MARGIN_PX, inset: Inset = ZERO_INSET): View {
  if (target.w <= 0 || target.h <= 0) {
    return {
      scale: CSS_PX_PER_MM,
      tx: margin + inset.left - target.x * CSS_PX_PER_MM,
      ty: margin + inset.top - target.y * CSS_PX_PER_MM,
    };
  }
  const availW = Math.max(size.w - 2 * margin - inset.left - inset.right, 1);
  const availH = Math.max(size.h - 2 * margin - inset.top - inset.bottom, 1);
  const scale = Math.min(availW / target.w, availH / target.h);
  // With a zero inset this is `(size - target * scale) / 2`, the centring the older call had.
  const x0 = inset.left + margin + (availW - target.w * scale) / 2;
  const y0 = inset.top + margin + (availH - target.h * scale) / 2;
  return {
    scale,
    tx: x0 - target.x * scale,
    ty: y0 - target.y * scale,
  };
}

/** Far enough out to see the fitted target small with room around it, and no further: past that the
 *  canvas is empty space and the way back is long. The caller passes the mat, not the machine bed,
 *  so the floor is not stuck at a few percent of a 3 m roll. */
export function minScaleFor(target: Bounds, size: Size, margin = FIT_MARGIN_PX, inset: Inset = ZERO_INSET): number {
  return Math.min(fitView(target, size, margin, inset).scale / 4, MAX_SCALE);
}

export function zoomAt(v: View, screen: Pt, factor: number, minScale: number): View {
  const scale = Math.min(Math.max(v.scale * factor, minScale), MAX_SCALE);
  const k = scale / v.scale;
  return { scale, tx: screen.x - (screen.x - v.tx) * k, ty: screen.y - (screen.y - v.ty) * k };
}

export function panBy(v: View, dx: number, dy: number): View {
  return { scale: v.scale, tx: v.tx + dx, ty: v.ty + dy };
}

export function worldToScreen(v: View, p: Pt): Pt {
  return { x: p.x * v.scale + v.tx, y: p.y * v.scale + v.ty };
}

export function screenToWorld(v: View, p: Pt): Pt {
  return { x: (p.x - v.tx) / v.scale, y: (p.y - v.ty) / v.scale };
}

export function viewMatrix(v: View): Affine6 {
  return [v.scale, 0, 0, v.scale, v.tx, v.ty];
}

export function zoomPercent(v: View): number {
  return Math.round((v.scale / CSS_PX_PER_MM) * 100);
}

export function wheelFactor(deltaY: number): number {
  return Math.exp(-deltaY * WHEEL_ZOOM_RATE);
}
