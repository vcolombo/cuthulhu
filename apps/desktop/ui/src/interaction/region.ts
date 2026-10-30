// SPDX-License-Identifier: GPL-3.0-or-later
import { apply, type Pt } from "../render/affine";
import type { Bounds, Scene, SceneNode } from "../render/hittest";
import { pathRings, type PathRing } from "../render/pathdata";

/** `contain`: the shape's outline lies entirely inside the region. `intersect`: the outline
 *  meets the region, or (a closed shape) the region sits in the fill. */
export type RegionMode = "contain" | "intersect";

const ELLIPSE_STEPS = 48;

/** Shapes whose geometry meets `region`. A node with no shape of its own is its world box:
 *  that is all the scene stored for it. The test uses the node's world matrix, which already
 *  includes its parents, so a shape under a transformed Group is hit where it is drawn. */
export function polygonHits(scene: Scene, region: Pt[], mode: RegionMode): number[] {
  if (region.length < 3) return [];
  const rb = boundsOf(region);
  const out: number[] = [];
  for (const n of scene.nodes) {
    if (!boundsOverlap(n.bounds, rb)) continue;
    const rings = nodeRings(n);
    if (mode === "contain" ? contained(rings, region) : overlaps(rings, region)) out.push(n.id);
  }
  return out;
}

/** The band as a ring, in the same winding the lasso uses, so both regions share one test. */
export function rectRing(b: Bounds): Pt[] {
  return [
    { x: b.x, y: b.y },
    { x: b.x + b.w, y: b.y },
    { x: b.x + b.w, y: b.y + b.h },
    { x: b.x, y: b.y + b.h },
  ];
}

function nodeRings(n: SceneNode): PathRing[] {
  const local = localRings(n);
  if (!local || !n.world) return [{ pts: rectRing(n.bounds), closed: true }];
  const world = n.world;
  return local.map((ring) => ({ closed: ring.closed, pts: ring.pts.map((p) => apply(world, p)) }));
}

function localRings(n: SceneNode): PathRing[] | null {
  const shape = n.shape;
  if (!shape) return null;
  if (shape.t === "rect") {
    return [{ pts: rectRing({ x: 0, y: 0, w: shape.w, h: shape.h }), closed: true }];
  }
  if (shape.t === "ellipse") {
    const pts: Pt[] = [];
    for (let i = 0; i < ELLIPSE_STEPS; i++) {
      const t = (i / ELLIPSE_STEPS) * Math.PI * 2;
      pts.push({ x: shape.rx + shape.rx * Math.cos(t), y: shape.ry + shape.ry * Math.sin(t) });
    }
    return [{ pts, closed: true }];
  }
  return pathRings(shape.d);
}

function contained(rings: PathRing[], region: Pt[]): boolean {
  const pts = rings.flatMap((r) => r.pts);
  if (pts.length === 0) return false;
  if (!pts.every((p) => pointInRing(region, p))) return false;
  // A shared edge is still inside. Only an edge that actually leaves the region — which a
  // concave loop can do between two inside vertices — fails containment.
  return !edgesCross(rings, [{ pts: region, closed: true }], false);
}

function overlaps(rings: PathRing[], region: Pt[]): boolean {
  if (rings.some((r) => r.pts.some((p) => pointInRing(region, p)))) return true;
  const regionRing = [{ pts: region, closed: true }];
  if (edgesCross(rings, regionRing, true)) return true;
  // A band sitting in the middle of a closed shape crosses no edge and holds no vertex.
  return rings.some((r) => r.closed) && region.some((p) => pointInFill(rings, p));
}

function edgesCross(a: PathRing[], b: PathRing[], touching: boolean): boolean {
  for (const ra of a) {
    for (const [a0, a1] of edgesOf(ra)) {
      for (const rb of b) {
        for (const [b0, b1] of edgesOf(rb)) {
          if (segmentsCross(a0, a1, b0, b1, touching)) return true;
        }
      }
    }
  }
  return false;
}

function edgesOf(ring: PathRing): [Pt, Pt][] {
  const pts = ring.pts;
  const n = pts.length;
  const last = ring.closed ? n : n - 1;
  const out: [Pt, Pt][] = [];
  for (let i = 0; i < last; i++) {
    const p = pts[i];
    const q = pts[(i + 1) % n];
    if (p.x !== q.x || p.y !== q.y) out.push([p, q]);
  }
  return out;
}

function pointInFill(rings: PathRing[], p: Pt): boolean {
  let inside = false;
  for (const ring of rings) {
    if (!ring.closed) continue;
    if (onBoundary(ring.pts, p)) return true;
    if (pointInRingOpen(ring.pts, p)) inside = !inside;
  }
  return inside;
}

/** Boundary counts as inside, so a shape that only touches the band is a hit. */
function pointInRing(ring: Pt[], p: Pt): boolean {
  if (ring.length < 3) return false;
  if (onBoundary(ring, p)) return true;
  return pointInRingOpen(ring, p);
}

function pointInRingOpen(ring: Pt[], p: Pt): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const a = ring[j];
    const b = ring[i];
    const crosses = a.y > p.y !== b.y > p.y && p.x < ((b.x - a.x) * (p.y - a.y)) / (b.y - a.y) + a.x;
    if (crosses) inside = !inside;
  }
  return inside;
}

function onBoundary(ring: Pt[], p: Pt): boolean {
  const n = ring.length;
  for (let i = 0; i < n; i++) {
    if (onSegment(ring[i], ring[(i + 1) % n], p)) return true;
  }
  return false;
}

function onSegment(a: Pt, b: Pt, p: Pt): boolean {
  const cross = (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x);
  if (Math.abs(cross) > 1e-8) return false;
  const dot = (p.x - a.x) * (b.x - a.x) + (p.y - a.y) * (b.y - a.y);
  if (dot < -1e-8) return false;
  const len2 = (b.x - a.x) ** 2 + (b.y - a.y) ** 2;
  return dot <= len2 + 1e-8;
}

function segmentsCross(a: Pt, b: Pt, c: Pt, d: Pt, touching: boolean): boolean {
  const d1 = cross(c, d, a);
  const d2 = cross(c, d, b);
  const d3 = cross(a, b, c);
  const d4 = cross(a, b, d);
  const eps = 1e-8;
  if (((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps)) && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))) return true;
  if (!touching) return false;
  if (Math.abs(d1) <= eps && onSegment(c, d, a)) return true;
  if (Math.abs(d2) <= eps && onSegment(c, d, b)) return true;
  if (Math.abs(d3) <= eps && onSegment(a, b, c)) return true;
  if (Math.abs(d4) <= eps && onSegment(a, b, d)) return true;
  return false;
}

function cross(o: Pt, a: Pt, b: Pt): number {
  return (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
}

function boundsOf(pts: Pt[]): Bounds {
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const p of pts) {
    x0 = Math.min(x0, p.x);
    y0 = Math.min(y0, p.y);
    x1 = Math.max(x1, p.x);
    y1 = Math.max(y1, p.y);
  }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

function boundsOverlap(a: Bounds, b: Bounds): boolean {
  return a.x <= b.x + b.w && a.x + a.w >= b.x && a.y <= b.y + b.h && a.y + a.h >= b.y;
}
