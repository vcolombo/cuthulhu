// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import {
  CSS_PX_PER_MM, CONTENT_FIT_MARGIN_PX, FIT_MARGIN_PX, MAX_SCALE, fitView, minScaleFor, panBy, screenToWorld,
  wheelFactor, worldToScreen, zoomAt, zoomPercent,
} from "./viewport";
import type { Pt } from "../render/affine";

const close = (a: Pt, b: Pt) => {
  expect(a.x).toBeCloseTo(b.x, 9);
  expect(a.y).toBeCloseTo(b.y, 9);
};

describe("viewport", () => {
  const artboard = { x: 0, y: 0, w: 300, h: 600 };
  const size = { w: 800, h: 600 };

  it("fit centres the artboard inside the margin", () => {
    const v = fitView(artboard, size);
    expect(v.scale).toBeCloseTo((600 - 2 * FIT_MARGIN_PX) / 600, 9); // height-bound here
    const tl = worldToScreen(v, { x: 0, y: 0 });
    const br = worldToScreen(v, { x: 300, y: 600 });
    expect(tl.y).toBeCloseTo(FIT_MARGIN_PX, 9);
    expect(br.y).toBeCloseTo(600 - FIT_MARGIN_PX, 9);
    expect((tl.x + br.x) / 2).toBeCloseTo(400, 9);
  });

  it("fit honours an artboard that does not start at the origin", () => {
    const v = fitView({ x: 50, y: -20, w: 100, h: 100 }, { w: 400, h: 400 });
    close(worldToScreen(v, { x: 100, y: 30 }), { x: 200, y: 200 });
  });

  it("screenToWorld inverts worldToScreen", () => {
    const v = { scale: 2.5, tx: -40, ty: 13 };
    close(screenToWorld(v, worldToScreen(v, { x: 7, y: -3 })), { x: 7, y: -3 });
  });

  it("zoomAt keeps the world point under the cursor where it was", () => {
    const v = fitView(artboard, size);
    const cursor = { x: 123, y: 456 };
    const z = zoomAt(v, cursor, 3, 0);
    expect(z.scale).toBeCloseTo(v.scale * 3, 9);
    close(screenToWorld(z, cursor), screenToWorld(v, cursor));
  });

  it("zoomAt clamps at both ends and still pins the cursor", () => {
    const v = { scale: 1, tx: 0, ty: 0 };
    expect(zoomAt(v, { x: 10, y: 10 }, 1e6, 0.5).scale).toBe(MAX_SCALE);
    const out = zoomAt(v, { x: 10, y: 10 }, 1e-6, 0.5);
    expect(out.scale).toBe(0.5);
    close(screenToWorld(out, { x: 10, y: 10 }), screenToWorld(v, { x: 10, y: 10 }));
  });

  it("the zoom floor is a quarter of fit", () => {
    expect(minScaleFor(artboard, size)).toBeCloseTo(fitView(artboard, size).scale / 4, 9);
  });

  it("fits a 12 in mat far larger than a 3 m bed, and the floor follows that target", () => {
    const window = { w: 900, h: 700 };
    const bed = { x: 0, y: 0, w: 330, h: 3000 };
    const mat = { x: 0, y: 0, w: 12 * 25.4, h: 12 * 25.4 };
    expect(fitView(mat, window).scale).toBeGreaterThan(fitView(bed, window).scale * 5);
    expect(minScaleFor(mat, window)).toBeGreaterThan(minScaleFor(bed, window));
  });

  it("keeps a ruler inset outside the margin without moving a zero inset", () => {
    const plain = fitView(artboard, size);
    const inset = fitView(artboard, size, FIT_MARGIN_PX, { left: 24, top: 24, right: 0, bottom: 0 });
    expect(plain.tx).toBeCloseTo((size.w - artboard.w * plain.scale) / 2, 9);
    const top = worldToScreen(inset, { x: 0, y: 0 });
    expect(top.y).toBeCloseTo(FIT_MARGIN_PX + 24, 9);
    expect(inset.scale).toBeLessThan(plain.scale);
  });

  it("centres the mat in the area the zoom bar does not cover", () => {
    const mat = { x: 0, y: 0, w: 304.8, h: 304.8 };
    const frame = { w: 900, h: 640 };
    const inset = { left: 24, top: 24, right: 0, bottom: 54 };
    const v = fitView(mat, frame, FIT_MARGIN_PX, inset);
    const tl = worldToScreen(v, { x: 0, y: 0 });
    const br = worldToScreen(v, { x: mat.w, y: mat.h });
    const free = {
      left: inset.left + FIT_MARGIN_PX,
      top: inset.top + FIT_MARGIN_PX,
      right: frame.w - inset.right - FIT_MARGIN_PX,
      bottom: frame.h - inset.bottom - FIT_MARGIN_PX,
    };
    expect(tl.x).toBeGreaterThanOrEqual(free.left - 1e-6);
    expect(tl.y).toBeGreaterThanOrEqual(free.top - 1e-6);
    expect(br.x).toBeLessThanOrEqual(free.right + 1e-6);
    expect(br.y).toBeLessThanOrEqual(free.bottom + 1e-6);
    expect((tl.x + br.x) / 2).toBeCloseTo((free.left + free.right) / 2, 6);
    expect((tl.y + br.y) / 2).toBeCloseTo((free.top + free.bottom) / 2, 6);
  });

  it("caps a few-millimetre target at the zoom ceiling and still centres it", () => {
    const target = { x: 0, y: 0, w: 0.01, h: 0.01 };
    const frame = { w: 2000, h: 2000 };
    const v = fitView(target, frame);
    expect(v.scale).toBe(MAX_SCALE);
    const tl = worldToScreen(v, { x: 0, y: 0 });
    const br = worldToScreen(v, { x: target.w, y: target.h });
    expect((tl.x + br.x) / 2).toBeCloseTo(frame.w / 2, 6);
    expect((tl.y + br.y) / 2).toBeCloseTo(frame.h / 2, 6);
  });

  it("gives Fit content more air than the mat fit", () => {
    const artwork = { x: 0, y: 0, w: 40, h: 20 };
    expect(fitView(artwork, size, CONTENT_FIT_MARGIN_PX).scale).toBeLessThan(fitView(artwork, size).scale);
  });

  it("a wheel delta and its negation cancel, and scrolling up zooms in", () => {
    expect(wheelFactor(37) * wheelFactor(-37)).toBeCloseTo(1, 12);
    expect(wheelFactor(-100)).toBeGreaterThan(1);
  });

  it("100% is actual size", () => {
    expect(zoomPercent({ scale: CSS_PX_PER_MM, tx: 0, ty: 0 })).toBe(100);
  });

  it("panBy moves the view and leaves the scale alone", () => {
    expect(panBy({ scale: 2, tx: 1, ty: 1 }, 10, -5)).toEqual({ scale: 2, tx: 11, ty: -4 });
  });
});
