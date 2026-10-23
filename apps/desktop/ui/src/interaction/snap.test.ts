// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { rotateAbout, translate } from "../render/affine";
import { keepLanded, SNAP_PX, snapMove, snapScale, snapTargets, type Targets } from "./snap";
import type { Box } from "./selectionBox";

// Rect A (moving) at 0..10, rect B at 30..40, on a Cameo-sized artboard. Tolerance 1 mm.
const A = { id: 1, bounds: { x: 0, y: 0, w: 10, h: 10 } };
const B = { id: 2, bounds: { x: 30, y: 0, w: 10, h: 10 } };
const artboard = { x: 0, y: 0, w: 330, h: 3000 };
const scene = { nodes: [A, B] };
const boxA: Box = { frame: translate(0, 0), w: 10, h: 10 };
const t = snapTargets(scene, [1], artboard);

describe("snapTargets", () => {
  it("leaves out the shapes being moved and includes the artboard's lines", () => {
    const xs = t.x.map((l) => l.v);
    expect(xs).toEqual(expect.arrayContaining([30, 35, 40, 0, 165, 330]));
    expect(xs).not.toContain(5); // A's centre: a selection never snaps to itself
  });

  it("sorts each axis by coordinate, so a pointer move is a binary search", () => {
    for (const lines of [t.x, t.y]) {
      for (let i = 1; i < lines.length; i++) expect(lines[i].v).toBeGreaterThanOrEqual(lines[i - 1].v);
    }
  });
});

describe("snapMove", () => {
  it("butts A's right edge against B when it lands within reach", () => {
    const r = snapMove(boxA, { x: 5, y: 5 }, { x: 24.6, y: 5 }, t, 1, false);
    expect(r.point.x).toBeCloseTo(25, 9);
    expect(r.point.y).toBeCloseTo(5, 9);
  });

  it("leaves an axis alone when the nearest line is out of reach", () => {
    expect(snapMove(boxA, { x: 5, y: 5 }, { x: 22, y: 5 }, t, 1, false).point.x).toBe(22);
  });

  it("prefers the centre line when two of the box's lines are equally close", () => {
    const tie: Targets = { x: [{ v: -0.5, lo: 0, hi: 10 }, { v: 5.5, lo: 0, hi: 10 }], y: [] };
    expect(snapMove(boxA, { x: 5, y: 5 }, { x: 5, y: 5 }, tie, 1, false).point.x).toBeCloseTo(5.5, 9);
  });

  it("with Shift's axis lock snaps only the free axis", () => {
    const r = snapMove(boxA, { x: 5, y: 5 }, { x: 24.6, y: 5.4 }, t, 1, true);
    expect(r.point.x).toBeCloseTo(25, 9);
    // Locked axis held at its start, so the snap cannot flip which axis the matrix locks to.
    expect(r.point.y).toBe(5);
  });

  it("draws one guide along the snapped line, spanning A and B", () => {
    const r = snapMove(boxA, { x: 5, y: 5 }, { x: 24.6, y: 5 }, t, 1, false);
    const g = r.guides.find((s) => s.axis === "x" && Math.abs(s.a.x - 30) < 1e-9);
    expect(g).toBeDefined();
    expect(Math.min(g!.a.y, g!.b.y)).toBeCloseTo(0, 9);
    expect(Math.max(g!.a.y, g!.b.y)).toBeCloseTo(10, 9);
  });

  it("reaches farther in mm when zoomed out, because the reach is CSS px", () => {
    // A dragged 17 mm stops 3 mm short of B: out of reach at 6 px/mm, within it at 1.5 px/mm.
    const at = (scale: number) => snapMove(boxA, { x: 5, y: 5 }, { x: 22, y: 5 }, t, SNAP_PX / scale, false).point.x;
    expect(at(6)).toBe(22);
    expect(at(1.5)).toBeCloseTo(25, 9);
  });
});

describe("snapScale", () => {
  it("snaps the edge an e handle moves", () => {
    const r = snapScale(boxA, "e", { x: 10, y: 5 }, { x: 29.6, y: 5 }, t, 1);
    expect(r.point.x).toBeCloseTo(30, 9);
    expect(r.point.y).toBe(5);
  });

  it("leaves x alone for an n handle, which moves only y", () => {
    const r = snapScale(boxA, "n", { x: 5, y: 0 }, { x: 5.3, y: -0.4 }, t, 1);
    expect(r.point.x).toBe(5.3);
    expect(r.point.y).toBeCloseTo(0, 9);
  });

  it("does not snap a rotated box, whose edges are not axis-aligned", () => {
    const rotated: Box = { frame: rotateAbout(Math.PI / 6, { x: 0, y: 0 }), w: 10, h: 10 };
    const cur = { x: 29.6, y: 5 };
    expect(snapScale(rotated, "e", { x: 10, y: 5 }, cur, t, 1)).toEqual({ point: cur, guides: [] });
  });
});

describe("snapping, the rest of the cases", () => {
  // C sits below A, 20 mm down, so A's bottom can meet C's top.
  const C = { id: 3, bounds: { x: 0, y: 20, w: 10, h: 10 } };
  const tc = snapTargets({ nodes: [A, C] }, [1], null);

  it("snaps on y too: A's bottom meets C's top", () => {
    expect(snapMove(boxA, { x: 5, y: 5 }, { x: 5, y: 14.6 }, tc, 1, false).point.y).toBeCloseTo(15, 9);
  });

  it("with Shift locked to y, snaps y and holds x at its start", () => {
    // The locked component is pinned so the snap cannot flip which axis the matrix locks to.
    const r = snapMove(boxA, { x: 5, y: 5 }, { x: 5.6, y: 14.6 }, tc, 1, true);
    expect(r.point.y).toBeCloseTo(15, 9);
    expect(r.point.x).toBe(5);
  });

  it("a corner handle snaps both axes at once", () => {
    const both = snapTargets({ nodes: [A, { id: 4, bounds: { x: 30, y: 30, w: 10, h: 10 } }] }, [1], null);
    const r = snapScale(boxA, "se", { x: 10, y: 10 }, { x: 29.6, y: 29.7 }, both, 1);
    expect(r.point).toEqual({ x: expect.closeTo(30, 9), y: expect.closeTo(30, 9) });
    expect(r.guides).toHaveLength(2);
  });

  it("a w handle, which moves the left edge, snaps it to a line on the left", () => {
    const left: Targets = { x: [{ v: -5, lo: 0, hi: 10 }], y: [] };
    expect(snapScale(boxA, "w", { x: 0, y: 5 }, { x: -4.6, y: 5 }, left, 1).point.x).toBeCloseTo(-5, 9);
  });

  it("a guide spans every target that shares its line", () => {
    const D = { id: 5, bounds: { x: 30, y: 50, w: 10, h: 10 } }; // shares B's left edge, far below
    const r = snapMove(boxA, { x: 5, y: 5 }, { x: 24.6, y: 5 }, snapTargets({ nodes: [A, B, D] }, [1], null), 1, false);
    const g = r.guides.find((s) => s.axis === "x")!;
    expect(Math.max(g.a.y, g.b.y)).toBeCloseTo(60, 9);
  });
});

describe("keepLanded", () => {
  const vertical = (x: number) => ({ axis: "x" as const, a: { x, y: 0 }, b: { x, y: 10 } });
  const horizontal = (y: number) => ({ axis: "y" as const, a: { x: 0, y }, b: { x: 10, y } });

  it("keeps a guide whose line the landed box touches, at an edge or a centre", () => {
    const landed = { x: 20, y: 0, w: 10, h: 10 };
    expect(keepLanded([vertical(30), vertical(25), horizontal(10)], landed)).toHaveLength(3);
  });

  it("drops a guide for a line the box did not land on", () => {
    // Shift sized the box from its other axis, or the minimum-size clamp held the edge back.
    expect(keepLanded([vertical(30), horizontal(30)], { x: 0, y: 0, w: 20, h: 20 })).toEqual([]);
  });
});

describe("keepLanded spans", () => {
  it("stretches a kept guide over the landed box, not where the box would have been", () => {
    // A guide along x = 30 from a target at y 0..10; the box landed lower, at y 40..50, as a Shift
    // move or a uniform scale can leave it. The guide must reach the box it lines up.
    const guide = { axis: "x" as const, a: { x: 30, y: 0 }, b: { x: 30, y: 10 } };
    const [g] = keepLanded([guide], { x: 20, y: 40, w: 10, h: 10 });
    expect([Math.min(g.a.y, g.b.y), Math.max(g.a.y, g.b.y)]).toEqual([0, 50]);
  });

  it("does not stretch over a box position the matrix never produced", () => {
    // snapMove's own guide covers only the targets; the box's extent is added from the landed box.
    const r = snapMove(boxA, { x: 5, y: 5 }, { x: 24.6, y: 20 }, t, 1, true); // Shift: locked to x
    const g = r.guides.find((s) => s.axis === "x")!;
    expect(Math.max(g.a.y, g.b.y)).toBeCloseTo(10, 9); // B's extent only, not the phantom y 15..25
  });
});

describe("guide orientation", () => {
  it("keeps a horizontal guide to a zero-width shape horizontal", () => {
    // A vertical straight line at x = 30 (w = 0) spanning y 0..20: its middle line is y = 10.
    // A guide spanning only that target has both ends at x = 30, which once read as vertical
    // (gate-1 review). A moves down 9.6 mm so its top meets y = 10.
    const line = { id: 6, bounds: { x: 30, y: 0, w: 0, h: 20 } };
    const r = snapMove(boxA, { x: 5, y: 5 }, { x: 5, y: 14.6 }, snapTargets({ nodes: [A, line] }, [1], null), 1, false);
    const kept = keepLanded(r.guides, { x: 0, y: 10, w: 10, h: 10 });
    const h = kept.find((g) => g.axis === "y");
    expect(h).toBeDefined();
    expect(h!.a.y).toBeCloseTo(10, 9);
    expect(h!.b.y).toBeCloseTo(10, 9);
    expect([Math.min(h!.a.x, h!.b.x), Math.max(h!.a.x, h!.b.x)]).toEqual([0, 30]);
  });
});
