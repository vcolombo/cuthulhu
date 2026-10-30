// SPDX-License-Identifier: GPL-3.0-or-later
import type { Bounds } from "./hittest";
import type { Pt } from "./affine";

const NUM = /-?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/g;

/** How far a cubic may bow off its chord before it is split. Selection tests the chords, so this
 *  is the most a curve can stick out of a band the test called empty. */
const FLAT_MM = 0.2;

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

export type PathRing = { pts: Pt[]; closed: boolean };

/** Outline of absolute M/L/C/Z path data — the subset `geometry::Path::from_svg` stores.
 *  Anything else returns null so the caller can fall back to the node's box instead of
 *  testing a path it only half-read. Curves are chords within `FLAT_MM`. */
export function pathRings(d: string): PathRing[] | null {
  const segs = parseAbsolute(d);
  if (!segs) return null;
  const rings: PathRing[] = [];
  let cur: Pt[] = [];
  let start: Pt = { x: 0, y: 0 };
  let here: Pt = start;
  let open = false;
  const flush = (closed: boolean) => {
    if (cur.length === 0) return;
    rings.push({ pts: cur, closed });
    cur = [];
    open = false;
  };
  for (const seg of segs) {
    if (seg.t === "M") {
      if (open || cur.length > 0) flush(false);
      cur = [seg.p];
      start = seg.p;
      here = seg.p;
      open = true;
    } else if (seg.t === "L") {
      cur.push(seg.p);
      here = seg.p;
      open = true;
    } else if (seg.t === "C") {
      flattenCubic(here, seg.c1, seg.c2, seg.p, FLAT_MM, cur);
      here = seg.p;
      open = true;
    } else {
      flush(true);
      here = start;
    }
  }
  if (cur.length > 0) flush(false);
  return rings.length > 0 ? rings : null;
}

type Seg =
  | { t: "M"; p: Pt }
  | { t: "L"; p: Pt }
  | { t: "C"; c1: Pt; c2: Pt; p: Pt }
  | { t: "Z" };

/** Same command rule as `geometry::Path::from_svg`: one absolute M, L, C, or Z, each with
 *  exactly the numbers it needs. A relative command or a short one is a miss, not a guess. */
function parseAbsolute(d: string): Seg[] | null {
  const segs: Seg[] = [];
  let nums: number[] = [];
  let cmd: string | null = null;
  let buf = "";
  const flush = (): boolean => {
    if (!cmd) return nums.length === 0 && buf === "";
    if (buf !== "") {
      const n = Number(buf);
      buf = "";
      if (!Number.isFinite(n)) return false;
      nums.push(n);
    }
    const need = cmd === "M" || cmd === "L" ? 2 : cmd === "C" ? 6 : cmd === "Z" ? 0 : -1;
    if (need < 0 || nums.length !== need) return false;
    if (cmd === "M") segs.push({ t: "M", p: { x: nums[0], y: nums[1] } });
    else if (cmd === "L") segs.push({ t: "L", p: { x: nums[0], y: nums[1] } });
    else if (cmd === "C") segs.push({ t: "C", c1: { x: nums[0], y: nums[1] }, c2: { x: nums[2], y: nums[3] }, p: { x: nums[4], y: nums[5] } });
    else segs.push({ t: "Z" });
    nums = [];
    return true;
  };
  for (const ch of `${d} `) {
    if ((ch === "e" || ch === "E") && buf !== "") buf += ch;
    else if (ch >= "A" && ch <= "Z") {
      if (!flush()) return null;
      cmd = ch;
    } else if (ch === "," || ch === " " || ch === "\t" || ch === "\n" || ch === "\r") {
      if (buf !== "") {
        const n = Number(buf);
        buf = "";
        if (!Number.isFinite(n)) return null;
        nums.push(n);
      }
    } else buf += ch;
  }
  if (!flush()) return null;
  return segs;
}

function flattenCubic(p0: Pt, p1: Pt, p2: Pt, p3: Pt, tol: number, out: Pt[]): void {
  const d1 = distToLine(p1, p0, p3);
  const d2 = distToLine(p2, p0, p3);
  if (Math.max(d1, d2) <= tol) {
    out.push(p3);
    return;
  }
  const mid = (a: Pt, b: Pt): Pt => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });
  const p01 = mid(p0, p1);
  const p12 = mid(p1, p2);
  const p23 = mid(p2, p3);
  const p012 = mid(p01, p12);
  const p123 = mid(p12, p23);
  const m = mid(p012, p123);
  flattenCubic(p0, p01, p012, m, tol, out);
  flattenCubic(m, p123, p23, p3, tol, out);
}

function distToLine(p: Pt, a: Pt, b: Pt): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy);
  if (len === 0) return Math.hypot(p.x - a.x, p.y - a.y);
  return Math.abs((p.x - a.x) * dy - (p.y - a.y) * dx) / len;
}
