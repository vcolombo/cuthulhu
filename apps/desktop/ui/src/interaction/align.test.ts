// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { alignMoves, distributeBlock, distributeMoves, type AlignMode, type Unit } from "./align";

const unit = (id: number, x: number, y: number, w: number, h: number): Unit => ({ ids: [id], bounds: { x, y, w, h } });
// Three units: A 0..10 × 0..10, B 20..24 × 5..25, C 40..60 × 2..6. Selection bounds 0..60 × 0..25.
const A = unit(1, 0, 0, 10, 10);
const B = unit(2, 20, 5, 4, 20);
const C = unit(3, 40, 2, 20, 4);
const artboard = { x: 0, y: 0, w: 330, h: 3000 };
const dx = (moves: ReturnType<typeof alignMoves>, id: number) => moves.find((m) => m.ids[0] === id)?.m[4] ?? 0;
const dy = (moves: ReturnType<typeof alignMoves>, id: number) => moves.find((m) => m.ids[0] === id)?.m[5] ?? 0;

describe("alignMoves, several units against their own bounds", () => {
  const cases: [AlignMode, number[], "x" | "y"][] = [
    ["left", [0, -20, -40], "x"],
    ["hcenter", [25, 8, -20], "x"], // centre of 0..60 is 30
    ["right", [50, 36, 0], "x"],
    ["top", [0, -5, -2], "y"],
    ["vmiddle", [7.5, -2.5, 8.5], "y"], // middle of 0..25 is 12.5
    ["bottom", [15, 0, 19], "y"],
  ];
  for (const [mode, expected, axis] of cases) {
    it(`${mode} moves each unit on ${axis} only`, () => {
      const moves = alignMoves([A, B, C], mode, artboard);
      const got = [1, 2, 3].map((id) => (axis === "x" ? dx(moves, id) : dy(moves, id)));
      expected.forEach((e, i) => expect(got[i]).toBeCloseTo(e, 9));
      for (const mv of moves) expect(axis === "x" ? mv.m[5] : mv.m[4]).toBe(0);
    });
  }

  it("leaves out units that would not move", () => {
    expect(alignMoves([A, B, C], "left", artboard).map((m) => m.ids[0])).toEqual([2, 3]);
  });
});

describe("alignMoves, one unit against the artboard", () => {
  it("centres a single unit on the mat", () => {
    const moves = alignMoves([B], "hcenter", artboard);
    expect(dx(moves, 2)).toBeCloseTo(165 - 22, 9);
  });

  it("does nothing with no units, or one unit and no artboard", () => {
    expect(alignMoves([], "left", artboard)).toEqual([]);
    expect(alignMoves([B], "left", null)).toEqual([]);
  });
});

describe("distributeMoves", () => {
  it("keeps the outermost units and makes the gaps equal", () => {
    // Span 0..60, sizes 10 + 4 + 20 = 34, so two gaps of 13: B's left goes to 23 (+3).
    const moves = distributeMoves([A, B, C], "x");
    expect(moves.map((m) => m.ids[0])).toEqual([2]);
    expect(dx(moves, 2)).toBeCloseTo(3, 9);
  });

  it("overlaps evenly when the units are longer than their span", () => {
    // A 0..10, D 2..32 (reaches furthest, so it stays), E 15..25 goes between: span 0..32, sizes
    // 50, gaps (32 - 50) / 2 = -9, so E's left goes to 10 - 9 = 1.
    const D = unit(4, 2, 0, 30, 1);
    const E = unit(5, 15, 0, 10, 1);
    const moves = distributeMoves([A, D, E], "x");
    expect(moves.map((m) => m.ids[0])).toEqual([5]);
    expect(dx(moves, 5)).toBeCloseTo(1 - 15, 9);
  });

  it("keeps the unit reaching furthest, not the one starting last", () => {
    // X 0..10, Y 5..50, Z 20..30: Y is the far end, Z goes between. Span 0..50, sizes 65, gaps
    // -7.5, so Z's left goes to 2.5 and Y stays where it is.
    const X = unit(9, 0, 0, 10, 1);
    const Y = unit(10, 5, 0, 45, 1);
    const Z = unit(11, 20, 0, 10, 1);
    const moves = distributeMoves([X, Y, Z], "x");
    expect(moves.map((m) => m.ids[0])).toEqual([11]);
    expect(dx(moves, 11)).toBeCloseTo(2.5 - 20, 9);
  });

  it("spaces the others inside a unit that spans them, with margins equal to the gaps", () => {
    // A 100 mm border holding two 10 mm pieces: 80 mm of room over three spaces, so 80/3 each.
    // The border stays; a span from the last-starting unit had thrown one piece past the other.
    const W = unit(12, 0, 0, 100, 1);
    const moves = distributeMoves([W, unit(13, 10, 0, 10, 1), unit(14, 20, 0, 10, 1)], "x");
    expect(moves.map((m) => m.ids[0])).toEqual([13, 14]);
    expect(dx(moves, 13)).toBeCloseTo(80 / 3 - 10, 9);
    expect(dx(moves, 14)).toBeCloseTo(10 + 160 / 3 - 20, 9);
  });

  it("moves nothing when more than one unit spans the rest", () => {
    // Stacked copies of one shape: no one of them is the frame.
    expect(distributeMoves([A, unit(16, 0, 0, 10, 10), unit(17, 2, 2, 4, 4)], "x")).toEqual([]);
  });

  it("leaves out a move that is not a finite number", () => {
    const broken = unit(15, NaN, 0, 10, 1);
    expect(alignMoves([A, broken], "left", artboard)).toEqual([]);
  });

  it("orders units with the same start by document order", () => {
    const P = unit(6, 0, 0, 2, 1);
    const Q = unit(7, 0, 0, 4, 1);
    const R = unit(8, 20, 0, 2, 1);
    // P first (earlier in the list), Q in the middle: span 0..22, sizes 8, gaps 7, so Q starts at 9.
    expect(dx(distributeMoves([P, Q, R], "x"), 7)).toBeCloseTo(9, 9);
  });

  it("needs three units", () => {
    expect(distributeMoves([A, B], "x")).toEqual([]);
  });

  it("works on y with the same rule", () => {
    // Tops 0..10, 12..22, 40..50: span 0..50, sizes 30, gaps 10, so the middle goes to 20 (+8).
    const p = [unit(100, 0, 0, 5, 10), unit(101, 0, 12, 5, 10), unit(102, 0, 40, 5, 10)];
    expect(dy(distributeMoves(p, "y"), 101)).toBeCloseTo(8, 9);
  });

  it("blocks the y case where a short piece would pass a longer one", () => {
    // Tops: A 0..10, C 2..6, B 5..25: gaps of -4.5 would put C's top at 5.5, past B's 5.
    expect(distributeBlock([A, B, C], "y")).toBe("crowded");
  });
});

describe("distributeBlock", () => {
  it("names why distribute cannot act, and is null for a frame or two ends", () => {
    const plate = unit(20, 0, 0, 100, 5);
    const p1 = unit(21, 10, 10, 10, 10);
    const p2 = unit(22, 40, 30, 10, 10);
    expect(distributeBlock([A, B], "x")).toBe("few");
    expect(distributeBlock([A, B, C], "x")).toBeNull();
    expect(distributeBlock([plate, p1, p2], "x")).toBeNull();
    expect(distributeBlock([A, unit(16, 0, 0, 10, 10), unit(17, 2, 2, 4, 4)], "x")).toBe("stacked");
  });

  it("blocks a frame too small for its pieces rather than pushing them through it", () => {
    // Three overlapping 50 mm pieces in a 100 mm border: spaced evenly they would leave it.
    const border = unit(30, 0, 0, 100, 100);
    const pieces = [unit(31, 0, 10, 50, 10), unit(32, 25, 30, 50, 10), unit(33, 50, 50, 50, 10)];
    expect(distributeBlock([border, ...pieces], "x")).toBe("tight");
    expect(distributeMoves([border, ...pieces], "x")).toEqual([]);
    expect(distributeBlock([border, ...pieces], "y")).toBeNull(); // 30 mm of pieces in 100 fits
  });

  it("judges each axis on its own: a frame on y, two ends on x", () => {
    // A tall plate on the left holding the pieces' heights but not their x positions.
    const plate = unit(40, 0, 0, 5, 100);
    const p1 = unit(41, 20, 10, 10, 10);
    const p2 = unit(42, 50, 60, 10, 10);
    expect(distributeBlock([p1, plate, p2], "x")).toBeNull();
    const moves = distributeMoves([p1, plate, p2], "y"); // 80 mm of room over three spaces
    expect(moves.map((m) => m.ids[0])).toEqual([41, 42]);
    expect(dy(moves, 41)).toBeCloseTo(80 / 3 - 10, 9);
    expect(moves.every((m) => m.m[4] === 0)).toBe(true);
  });

  it("treats a piece flush with its border within float noise as inside it", () => {
    // 0.1 + 0.2 is 0.30000000000000004: compared exactly, this piece "reached further" than the
    // 0.3 plate, and the plate stopped being the frame.
    const plate = unit(50, 0, 0, 0.3, 1);
    const inner = unit(51, 0.05, 0, 0.05, 1);
    const flush = unit(52, 0.1, 0, 0.2, 1);
    expect(distributeBlock([plate, inner, flush], "x")).toBeNull();
    expect(distributeMoves([plate, inner, flush], "x").map((m) => m.ids[0])).toEqual([51, 52]);
  });

  it("measures the far end against the true maximum, so tolerances cannot chain", () => {
    // Copilot on #301: each end within 1e-6 of the last pick walked the far end back to
    // 100.00000075, which made the 0..100 unit look like a frame. Against the true maximum
    // (100.0000015) nothing spans, and the three distribute between two ends.
    // Read as two ends, these overlap too deeply to space ("crowded"); the chained far end had
    // read them as a frame too small for its pieces ("tight").
    const units = [unit(70, 0, 0, 100, 1), unit(71, 10, 0, 90.0000015, 1), unit(72, 20, 0, 80.00000075, 1)];
    expect(distributeBlock(units, "x")).toBe("crowded");
  });

  it("keeps the unit that truly reaches furthest, not one ending just short of it", () => {
    // CodeRabbit on #301: within tolerance of the far edge is not the far edge. 20..100 stays and
    // the 99.9..99.9999995 piece is the one placed between.
    const units = [unit(80, 0, 0, 10, 1), unit(81, 20, 0, 80, 1), unit(82, 99.9, 0, 0.0999995, 1)];
    expect(distributeMoves(units, "x").map((m) => m.ids[0])).toEqual([82]);
  });

  it("blocks overlap so deep that pieces would pass each other", () => {
    // Copilot on #301: 0..10, 1..100 and 50..70 give a gap of -14.5, which put the third piece at
    // -4.5, ahead of the first; the next click then picked a different first anchor.
    const units = [unit(90, 0, 0, 10, 1), unit(91, 1, 0, 99, 1), unit(92, 50, 0, 20, 1)];
    expect(distributeBlock(units, "x")).toBe("crowded");
    expect(distributeMoves(units, "x")).toEqual([]);
  });

  it("lands an overlapping layout in order with equal gaps between neighbours", () => {
    // A 0..10, D 2..32, E 15..25: gap -9, shallow enough that nothing passes anything.
    const units = [unit(1, 0, 0, 10, 1), unit(4, 2, 0, 30, 1), unit(5, 15, 0, 10, 1)];
    const moved = new Map(distributeMoves(units, "x").map((m) => [m.ids[0], m.m[4]]));
    const landed = units
      .map((u) => ({ x: u.bounds.x + (moved.get(u.ids[0]) ?? 0), w: u.bounds.w }))
      .sort((p, q) => p.x - q.x);
    const gaps = landed.slice(1).map((u, i) => u.x - (landed[i].x + landed[i].w));
    gaps.forEach((g) => expect(g).toBeCloseTo(-9, 9));
    expect(landed[0].x).toBe(0); // the first anchor stayed first
  });

  it("blocks overlap that leaves two pieces starting on the same edge", () => {
    // Copilot on #301: in document order B 5..25, A 0..10, C 10..30. The gap is -10, so B lands on
    // 0, level with A; the tie then goes to B by document order and the next click moves A.
    const units = [unit(110, 5, 0, 20, 1), unit(111, 0, 0, 10, 1), unit(112, 10, 0, 20, 1)];
    expect(distributeBlock(units, "x")).toBe("crowded");
  });

  it("blocks overlap that would carry a middle piece past the far end", () => {
    // A 0..10, B 1..30, C 25..32 (the far end, 7 long): the gap is -7, as long as C, so B would end
    // level with C; any deeper and it would pass C and become the far end on the next click.
    const units = [unit(120, 0, 0, 10, 1), unit(121, 1, 0, 29, 1), unit(122, 25, 0, 7, 1)];
    expect(distributeBlock(units, "x")).toBe("crowded");
  });

  it("lands every allowed layout so that a second click moves nothing", () => {
    const land = (us: Unit[]) => {
      const by = new Map(distributeMoves(us, "x").map((m) => [m.ids[0], m.m[4]]));
      return us.map((u) => ({ ...u, bounds: { ...u.bounds, x: u.bounds.x + (by.get(u.ids[0]) ?? 0) } }));
    };
    const layouts = [
      [unit(1, 0, 0, 10, 1), unit(4, 2, 0, 30, 1), unit(5, 15, 0, 10, 1)], // overlapping
      [unit(130, 0, 0, 1, 1), unit(131, 10, 0, 90, 1), unit(132, 11, 0, 1, 1)], // inside the far end
      [A, B, C],
    ];
    for (const us of layouts) expect(distributeMoves(land(us), "x")).toEqual([]);
  });

  it("blocks two units with exactly the same span as stacked", () => {
    expect(distributeBlock([unit(60, 0, 0, 100, 1), unit(61, 0, 0, 100, 1), unit(62, 40, 0, 10, 1)], "x")).toBe("stacked");
  });

  it("blocks a frame inside the frame rather than spacing it as one more piece", () => {
    // A backing plate around a weed border around three letters, all selected: spaced as a piece,
    // the border went to 14..114 and the letters to 128..186, outside it.
    const plate = unit(140, 0, 0, 200, 200);
    const border = unit(141, 50, 50, 100, 100);
    const letters = [unit(142, 60, 70, 10, 10), unit(143, 90, 70, 10, 10), unit(144, 120, 70, 10, 10)];
    for (const axis of ["x", "y"] as const) {
      expect(distributeBlock([plate, border, ...letters], axis)).toBe("nested");
      expect(distributeMoves([plate, border, ...letters], axis)).toEqual([]);
    }
    // Without the plate the border is the frame, as before.
    expect(distributeBlock([border, ...letters], "x")).toBeNull();
  });

  it("blocks a line of no width that fills a frame level with a neighbour", () => {
    // The pieces fill the frame, so the margin is 0 and the line lands on a neighbour's start; the
    // next click took document order from there and moved it again, three clicks in all.
    const units = [unit(150, 0, 0, 20, 1), unit(151, 5, 0, 10, 1), unit(152, 10, 0, 10, 1), unit(153, 2, 0, 0, 1)];
    expect(distributeBlock(units, "x")).toBe("crowded");
    // With room to spare the line has a margin of its own and lands once.
    const roomy = [unit(160, 0, 0, 40, 1), unit(161, 5, 0, 10, 1), unit(162, 10, 0, 10, 1), unit(163, 2, 0, 0, 1)];
    const by = new Map(distributeMoves(roomy, "x").map((m) => [m.ids[0], m.m[4]]));
    const landed = roomy.map((u) => ({ ...u, bounds: { ...u.bounds, x: u.bounds.x + (by.get(u.ids[0]) ?? 0) } }));
    expect(distributeMoves(landed, "x")).toEqual([]);
  });

  it("finds a frame that shares its start with another unit, in either document order", () => {
    // 100 - 15 = 85 mm of room over three spaces: the narrow piece goes to 85/3, the other after it.
    const narrow = unit(23, 0, 0, 5, 1);
    const plate = unit(24, 0, 0, 100, 1);
    const piece = unit(25, 10, 0, 10, 1);
    for (const units of [[narrow, plate, piece], [plate, narrow, piece]]) {
      const moves = distributeMoves(units, "x");
      expect(moves.map((m) => m.ids[0])).toEqual([23, 25]);
      expect(dx(moves, 23)).toBeCloseTo(85 / 3, 9);
      expect(dx(moves, 25)).toBeCloseTo(85 / 3 + 5 + 85 / 3 - 10, 9);
    }
  });
});

describe("distribute on a very large selection", () => {
  it("does not overflow the engine's argument limit", () => {
    // Copilot on #301: Math.max(...ends) passed one argument per unit, which throws RangeError
    // past the engine's limit, and the panel computes this while it renders.
    const many = Array.from({ length: 300_000 }, (_, i) => unit(i, i * 2, 0, 1, 1));
    expect(() => distributeBlock(many, "x")).not.toThrow();
  });
});
