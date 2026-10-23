// SPDX-License-Identifier: GPL-3.0-or-later
import { apply, axisLengths, compose, IDENTITY, invert, rotateAbout, scaleAbout, translate, type Pt } from "../render/affine";
import type { Matrix } from "./transform";
import { boxCenter, HANDLE_UNIT, handleLocal, type Box, type HandleKind, type ScaleHandle } from "./selectionBox";

export type Modifiers = { shift: boolean; alt: boolean };

/** Smallest a scale handle will take an axis, in world mm. Below it the shape is not cuttable,
 *  and crossing zero would mirror it — a mirrored cut should be a command, not an overshoot. */
export const MIN_SIZE_MM = 0.1;
export const ROTATE_SNAP_RAD = Math.PI / 12; // 15°

/** The whole gesture as one world-space matrix: the preview applies it and the commit sends it,
 *  so what the operator releases is what is saved. */
export function gestureMatrix(kind: HandleKind, box: Box, start: Pt, cur: Pt, mods: Modifiers): Matrix {
  if (kind === "move") return moveMatrix(start, cur, mods.shift);
  if (kind === "rotate") return rotateMatrix(box, start, cur, mods.shift);
  return scaleMatrix(kind, box, start, cur, mods);
}

function moveMatrix(start: Pt, cur: Pt, shift: boolean): Matrix {
  const dx = cur.x - start.x;
  const dy = cur.y - start.y;
  if (!shift) return translate(dx, dy);
  return Math.abs(dx) >= Math.abs(dy) ? translate(dx, 0) : translate(0, dy);
}

function rotateMatrix(box: Box, start: Pt, cur: Pt, shift: boolean): Matrix {
  const c = boxCenter(box);
  const swept = Math.atan2(cur.y - c.y, cur.x - c.x) - Math.atan2(start.y - c.y, start.x - c.x);
  const angle = shift ? Math.round(swept / ROTATE_SNAP_RAD) * ROTATE_SNAP_RAD : swept;
  return rotateAbout(angle, c);
}

/** Works in the box's own frame — scale there about the anchor, then carry it back to world —
 *  so a rotated box scales along its edges, and an axis-aligned one is just the special case. */
function scaleMatrix(h: ScaleHandle, box: Box, start: Pt, cur: Pt, mods: Modifiers): Matrix {
  const inv = invert(box.frame);
  if (!inv) return IDENTITY;
  const ps = apply(inv, start);
  const pc = apply(inv, cur);
  const unit = HANDLE_UNIT[h];
  const grab = handleLocal(box, h);
  const anchor = mods.alt ? { x: box.w / 2, y: box.h / 2 } : { x: (1 - unit.x) * box.w, y: (1 - unit.y) * box.h };
  const [lx, ly] = axisLengths(box.frame);
  const worldW = box.w * lx;
  const worldH = box.h * ly;
  const movesX = unit.x !== 0.5;
  const movesY = unit.y !== 0.5;
  let sx = movesX ? axisScale(grab.x, pc.x - ps.x, anchor.x, worldW) : 1;
  let sy = movesY ? axisScale(grab.y, pc.y - ps.y, anchor.y, worldH) : 1;
  if (mods.shift) {
    const s = !movesX ? sy : !movesY ? sx : Math.abs(sx - 1) >= Math.abs(sy - 1) ? sx : sy;
    const floor = Math.max(worldW > 0 ? MIN_SIZE_MM / worldW : 0, worldH > 0 ? MIN_SIZE_MM / worldH : 0);
    sx = Math.max(s, floor);
    sy = sx;
  }
  return compose(compose(inv, scaleAbout(sx, sy, anchor)), box.frame);
}

/** Factor that moves the handle at `grab` by `delta` while `anchor` stays put. Measured from the
 *  handle rather than the press point, so grabbing a handle off-centre does not skew the result. */
function axisScale(grab: number, delta: number, anchor: number, worldLen: number): number {
  const span = grab - anchor;
  if (span === 0 || worldLen === 0) return 1; // no extent along this axis to scale
  return Math.max((span + delta) / span, MIN_SIZE_MM / worldLen);
}
