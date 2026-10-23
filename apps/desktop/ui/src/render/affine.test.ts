// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { apply, IDENTITY, invert, isIdentity, rotateAbout, scaleAbout, compose, transformBounds, translate, type Pt } from "./affine";

const close = (a: Pt, b: Pt) => {
  expect(a.x).toBeCloseTo(b.x, 9);
  expect(a.y).toBeCloseTo(b.y, 9);
};

describe("affine", () => {
  it("compose applies the left matrix first, as the Rust Affine::then does", () => {
    // Scale-then-translate and translate-then-scale differ; this pins which one `compose` means.
    const m = compose(scaleAbout(2, 2, { x: 0, y: 0 }), translate(10, 0));
    close(apply(m, { x: 1, y: 1 }), { x: 12, y: 2 });
  });

  it("invert undoes a rotate, a non-uniform scale and a translate", () => {
    const m = compose(compose(rotateAbout(0.7, { x: 3, y: 4 }), scaleAbout(2, 0.5, { x: 0, y: 0 })), translate(5, -2));
    const inv = invert(m);
    expect(inv).not.toBeNull();
    close(apply(inv!, apply(m, { x: 7, y: 11 })), { x: 7, y: 11 });
  });

  it("invert refuses a singular matrix", () => {
    expect(invert([0, 0, 0, 1, 0, 0])).toBeNull();
  });

  it("scaleAbout and rotateAbout keep their centre fixed", () => {
    close(apply(scaleAbout(3, 0.25, { x: 4, y: 9 }), { x: 4, y: 9 }), { x: 4, y: 9 });
    close(apply(rotateAbout(1.1, { x: 4, y: 9 }), { x: 4, y: 9 }), { x: 4, y: 9 });
  });

  it("rotateAbout turns +x towards +y, which is clockwise on a y-down screen", () => {
    close(apply(rotateAbout(Math.PI / 2, { x: 0, y: 0 }), { x: 1, y: 0 }), { x: 0, y: 1 });
  });

  it("transformBounds boxes every corner of a rotated rect", () => {
    const b = transformBounds(rotateAbout(Math.PI / 4, { x: 5, y: 5 }), { x: 0, y: 0, w: 10, h: 10 });
    const half = 5 * Math.SQRT2;
    expect(b.x).toBeCloseTo(5 - half, 9);
    expect(b.w).toBeCloseTo(2 * half, 9);
  });

  it("isIdentity tolerates float noise and nothing more", () => {
    expect(isIdentity(IDENTITY)).toBe(true);
    expect(isIdentity([1, 0, 0, 1, 1e-12, 0])).toBe(true);
    expect(isIdentity(translate(0.01, 0))).toBe(false);
  });
});
