// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { apply, IDENTITY, rotateAbout, transformBounds, translate, type Pt } from "../render/affine";
import { handleAt, handleWorld, selectionBounds, selectionBox } from "./selectionBox";

const close = (a: Pt, b: Pt) => {
  expect(a.x).toBeCloseTo(b.x, 9);
  expect(a.y).toBeCloseTo(b.y, 9);
};

const r30 = rotateAbout(Math.PI / 6, { x: 0, y: 0 });
const rotated = {
  id: 1,
  local: { x: 0, y: 0, w: 10, h: 4 },
  world: r30,
  bounds: transformBounds(r30, { x: 0, y: 0, w: 10, h: 4 }),
};

describe("selectionBox", () => {
  it("one node's box is its own oriented box, so handles sit on its edges", () => {
    const box = selectionBox({ nodes: [rotated] }, [1])!;
    close(handleWorld(box, "ne"), apply(r30, { x: 10, y: 0 }));
    close(handleWorld(box, "s"), apply(r30, { x: 5, y: 4 }));
  });

  it("a path whose local box does not start at 0,0 still gets handles on it", () => {
    const n = { id: 2, local: { x: 3, y: 5, w: 2, h: 2 }, world: IDENTITY, bounds: { x: 3, y: 5, w: 2, h: 2 } };
    close(handleWorld(selectionBox({ nodes: [n] }, [2])!, "nw"), { x: 3, y: 5 });
  });

  it("several nodes share their axis-aligned union box", () => {
    const a = { id: 1, bounds: { x: 0, y: 0, w: 2, h: 2 } };
    const b = { id: 2, bounds: { x: 5, y: 3, w: 1, h: 4 } };
    const box = selectionBox({ nodes: [a, b] }, [1, 2])!;
    close(handleWorld(box, "nw"), { x: 0, y: 0 });
    close(handleWorld(box, "se"), { x: 6, y: 7 });
  });

  it("no selection, no box", () => {
    expect(selectionBox({ nodes: [rotated] }, [])).toBeNull();
  });
});

describe("selectionBounds", () => {
  it("is the axis-aligned world box of the shapes that are selected", () => {
    const a = { id: 1, bounds: { x: 1, y: 2, w: 3, h: 4 } };
    const b = { id: 2, bounds: { x: 10, y: -1, w: 2, h: 2 } };
    expect(selectionBounds({ nodes: [a, b] }, [1])).toEqual({ x: 1, y: 2, w: 3, h: 4 });
    expect(selectionBounds({ nodes: [a, b] }, [2, 1])).toEqual({ x: 1, y: -1, w: 11, h: 7 });
    expect(selectionBounds({ nodes: [a, b] }, [9])).toBeNull();
    expect(selectionBounds({ nodes: [a] }, [])).toBeNull();
  });

  it("does not overflow the engine's argument limit", () => {
    // The status bar used to spread every selected box into Math.min. V8 throws
    // RangeError past ~125k arguments, and Cmd-A selects every shape, so that
    // render took the window down. The align suite pins the same limit at 300k.
    const nodes = Array.from({ length: 300_000 }, (_, i) => ({
      id: i,
      bounds: { x: i, y: 0, w: 1, h: 1 },
    }));
    expect(selectionBounds({ nodes }, nodes.map((n) => n.id))).toEqual({
      x: 0, y: 0, w: 300_000, h: 1,
    });
  });
});

describe("handleAt", () => {
  const box = { frame: translate(10, 10), w: 20, h: 10 };

  it("finds handles first, then the inside, then the rotate zone", () => {
    expect(handleAt(box, { x: 30.5, y: 20.5 }, 1, 3)).toBe("se");
    expect(handleAt(box, { x: 20, y: 15 }, 1, 3)).toBe("move");
    expect(handleAt(box, { x: 32, y: 22 }, 1, 3)).toBe("rotate");
    expect(handleAt(box, { x: 50, y: 50 }, 1, 3)).toBeNull();
  });

  it("finds a rotated box's handles where they are drawn", () => {
    const b = selectionBox({ nodes: [rotated] }, [1])!;
    expect(handleAt(b, apply(r30, { x: 10, y: 4 }), 0.5, 2)).toBe("se");
  });

  it("a zero-height box (a straight path) still has an inside to drag", () => {
    const line = { frame: translate(0, 0), w: 10, h: 0 };
    expect(handleAt(line, { x: 3, y: 0 }, 0.5, 1)).toBe("move");
  });

  it("offers no handle that could only scale a zero-length axis", () => {
    // On a straight horizontal path the n and s handles sit on its midpoint and only scale its
    // height, which is zero: claiming the press there advertised a resize that does nothing and
    // made the midpoint undraggable (Copilot on #298). The ends still scale its length.
    const line = { frame: translate(0, 0), w: 10, h: 0 };
    expect(handleAt(line, { x: 5, y: 0 }, 0.5, 1)).toBe("move");
    expect(handleAt(line, { x: 10, y: 0 }, 0.5, 1)).not.toBe("move");
  });
});
