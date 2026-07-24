// SPDX-License-Identifier: GPL-3.0-or-later
import type { Bounds } from "./hittest";

const NUM = /-?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/g;

/** Axis-aligned bounds over every coordinate in absolute M/L/C/Z path data.
 *  Control points are included, so curve bounds are conservative (never smaller
 *  than the true ink). Returns null if the data contains no coordinates. */
export function pathBounds(d: string): Bounds | null {
  const nums = d.match(NUM);
  if (!nums || nums.length < 2) return null;
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (let i = 0; i + 1 < nums.length; i += 2) {
    const x = Number(nums[i]), y = Number(nums[i + 1]);
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (x > maxX) maxX = x;
    if (y > maxY) maxY = y;
  }
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
}
