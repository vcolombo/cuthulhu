// SPDX-License-Identifier: GPL-3.0-or-later
import type { Affine6, Bounds } from "./hittest";

export type Pt = { x: number; y: number };

export const IDENTITY: Affine6 = [1, 0, 0, 1, 0, 0];

/** Mirrors crates/geometry/src/affine.rs's `Affine::then`: apply `self`, then `other`. The Rust
 *  order on purpose — a matrix built here crosses IPC as a `commit_transforms` move's `m` and is composed
 *  there by the same rule, so the two sides cannot disagree about what a gesture meant.
 *
 *  Not named `then` like its Rust twin: a module exporting `then` is a thenable, so any dynamic
 *  `import()` of it calls this function with Promise callbacks and throws instead of resolving. */
export function compose(self: Affine6, other: Affine6): Affine6 {
  const [a1, b1, c1, d1, e1, f1] = self;
  const [a2, b2, c2, d2, e2, f2] = other;
  return [
    a2 * a1 + c2 * b1,
    b2 * a1 + d2 * b1,
    a2 * c1 + c2 * d1,
    b2 * c1 + d2 * d1,
    a2 * e1 + c2 * f1 + e2,
    b2 * e1 + d2 * f1 + f2,
  ];
}

export function apply(m: Affine6, p: Pt): Pt {
  const [a, b, c, d, e, f] = m;
  return { x: a * p.x + c * p.y + e, y: b * p.x + d * p.y + f };
}

/** `null` for a singular matrix: a node scaled to nothing has no inside to click or drag. */
export function invert(m: Affine6): Affine6 | null {
  const [a, b, c, d, e, f] = m;
  const det = a * d - b * c;
  if (det === 0) return null;
  const ia = d / det;
  const ib = -b / det;
  const ic = -c / det;
  const id = a / det;
  return [ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)];
}

export function translate(dx: number, dy: number): Affine6 {
  return [1, 0, 0, 1, dx, dy];
}

export function scaleAbout(sx: number, sy: number, c: Pt): Affine6 {
  return [sx, 0, 0, sy, c.x - sx * c.x, c.y - sy * c.y];
}

export function rotateAbout(rad: number, c: Pt): Affine6 {
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  return [cos, sin, -sin, cos, c.x - cos * c.x + sin * c.y, c.y - sin * c.x - cos * c.y];
}

/** How long one unit along local x, and along local y, is after `m`. Converts a world-space
 *  tolerance or minimum size into a node's own units axis by axis, under any scale or rotation. */
export function axisLengths(m: Affine6): [number, number] {
  return [Math.hypot(m[0], m[1]), Math.hypot(m[2], m[3])];
}

export function isIdentity(m: Affine6, eps = 1e-9): boolean {
  return m.every((v, i) => Math.abs(v - IDENTITY[i]) <= eps);
}

/** Axis-aligned box of `b` after `m`. All four corners, not two, so rotation is covered. */
export function transformBounds(m: Affine6, b: Bounds): Bounds {
  const corners = [
    apply(m, { x: b.x, y: b.y }),
    apply(m, { x: b.x + b.w, y: b.y }),
    apply(m, { x: b.x, y: b.y + b.h }),
    apply(m, { x: b.x + b.w, y: b.y + b.h }),
  ];
  const xs = corners.map((p) => p.x);
  const ys = corners.map((p) => p.y);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
}
