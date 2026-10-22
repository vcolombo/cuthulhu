// SPDX-License-Identifier: GPL-3.0-or-later
// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type { Scene } from "../render/hittest";
import { vinylDisplay } from "../render/vinyl";
import { cssColor, cutPreviewViewport, CutPreview, PREVIEW_BADGE_PX, PREVIEW_GAP_PX, type PreviewPass } from "./CutPreview";

describe("cutPreviewViewport", () => {
  it("keeps the shape and its order badge inside the frame", () => {
    const canvas = { w: 400, h: 300 };
    const content = { x: 0, y: 0, w: 40, h: 10 };
    const vp = cutPreviewViewport(content, { x: 0, y: 0, w: 330, h: 3000 }, canvas);
    const left = content.x * vp.scale + vp.tx;
    const top = content.y * vp.scale + vp.ty;
    const right = (content.x + content.w) * vp.scale + vp.tx;
    const bottom = (content.y + content.h) * vp.scale + vp.ty;
    expect(left - PREVIEW_BADGE_PX).toBeGreaterThanOrEqual(PREVIEW_GAP_PX - 0.5);
    expect(top - PREVIEW_BADGE_PX).toBeGreaterThanOrEqual(PREVIEW_GAP_PX - 0.5);
    expect(right + PREVIEW_BADGE_PX).toBeLessThanOrEqual(canvas.w - PREVIEW_GAP_PX + 0.5);
    expect(bottom + PREVIEW_BADGE_PX).toBeLessThanOrEqual(canvas.h - PREVIEW_GAP_PX + 0.5);
  });
});

describe("cssColor", () => {
  it("unpacks 0xRRGGBBAA into an rgba() string", () => {
    expect(cssColor(0xff0000ff)).toBe("rgba(255, 0, 0, 1)");
    expect(cssColor(0x00ff0080)).toBe("rgba(0, 255, 0, 0.5019607843137255)");
  });
  it("handles zero alpha", () => {
    expect(cssColor(0x00000000)).toBe("rgba(0, 0, 0, 0)");
  });
});

type Draw = { op: "fill" | "fillText"; style: string; text?: string };

function recordingContext(canvas: HTMLCanvasElement, draws: Draw[], transforms: number[][]) {
  const ctx = {
    canvas,
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    globalAlpha: 1,
    font: "",
    textAlign: "start",
    textBaseline: "alphabetic",
    setTransform(...args: number[]) { transforms.push(args); },
    clearRect() {},
    fillRect() {},
    strokeRect() {},
    beginPath() {},
    moveTo() {},
    lineTo() {},
    arc() {},
    setLineDash() {},
    // The disc behind a number is fill() with no path. A shape fill hands over the path.
    fill(path?: unknown) {
      if (path) draws.push({ op: "fill", style: String(ctx.fillStyle) });
    },
    stroke() {},
    fillText(text: string) {
      draws.push({ op: "fillText", style: String(ctx.fillStyle), text });
    },
  };
  return ctx;
}

function rect(id: number, x: number): Scene["nodes"][number] {
  return {
    id,
    bounds: { x, y: 0, w: 10, h: 10 },
    shape: { t: "rect", w: 10, h: 10 },
    world: [1, 0, 0, 1, x, 0],
  };
}

// happy-dom provides DOMMatrix and not Path2D. The preview only needs a path it can hand to fill().
if (typeof globalThis.Path2D === "undefined") {
  globalThis.Path2D = class Path2D {
    rect() {}
    ellipse() {}
    addPath() {}
  } as unknown as typeof Path2D;
}

describe("CutPreview drawing", () => {
  let root: Root;
  let el: HTMLDivElement;
  const original = HTMLCanvasElement.prototype.getContext;

  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  afterEach(() => {
    HTMLCanvasElement.prototype.getContext = original;
    act(() => root.unmount());
    el.remove();
  });

  it("fills each colour pass from vinylDisplay and numbers every shape in cut order", async () => {
    const draws: Draw[] = [];
    const transforms: number[][] = [];
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement) {
      return recordingContext(this, draws, transforms);
    } as unknown as typeof HTMLCanvasElement.prototype.getContext;
    const scene: Scene = { nodes: [rect(1, 0), rect(2, 20), rect(3, 40)] };
    const passes: PreviewPass[] = [
      { key: "color:ff0000ff", nodeIds: [1, 2], starts: [null, null], enabled: true, weed: [] },
      { key: "color:00ff00ff", nodeIds: [3], starts: [null], enabled: true, weed: [] },
    ];
    el = document.createElement("div");
    document.body.appendChild(el);
    root = createRoot(el);
    await act(async () => {
      root.render(createElement(CutPreview, { scene, artboard: { x: 0, y: 0, w: 100, h: 100 }, passes, travel: [], theme: "light" }));
    });

    const fills = draws.filter((d) => d.op === "fill").map((d) => d.style);
    expect(fills).toEqual([
      vinylDisplay(0xff0000ff, "light").fill,
      vinylDisplay(0xff0000ff, "light").fill,
      vinylDisplay(0x00ff00ff, "light").fill,
    ]);
    // A badge per pass would read "1", "1", "2": the second shape of the red pass is still pass 1.
    expect(draws.filter((d) => d.op === "fillText").map((d) => d.text)).toEqual(["1", "2", "3"]);

    // The fit the effect actually used. A 16px margin leaves the badge on the frame and still
    // passes the fill and number checks above.
    const canvas = el.querySelector("canvas");
    expect(canvas).toBeTruthy();
    const expected = cutPreviewViewport(
      { x: 0, y: 0, w: 50, h: 10 },
      { x: 0, y: 0, w: 100, h: 100 },
      { w: canvas?.width ?? 0, h: canvas?.height ?? 0 },
    );
    const fit = transforms.find((args) => args[0] !== 1);
    expect(fit?.[0]).toBeCloseTo(expected.scale);
    expect(fit?.[4]).toBeCloseTo(expected.tx);
    expect(fit?.[5]).toBeCloseTo(expected.ty);
  });

  it("numbers only the shapes an enabled pass will cut", async () => {
    const draws: Draw[] = [];
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement) {
      return recordingContext(this, draws, []);
    } as unknown as typeof HTMLCanvasElement.prototype.getContext;
    const scene: Scene = { nodes: [rect(1, 0), rect(2, 20), rect(3, 40)] };
    const passes: PreviewPass[] = [
      { key: "color:ff0000ff", nodeIds: [1, 2], starts: [null, null], enabled: false, weed: [] },
      { key: "color:00ff00ff", nodeIds: [3], starts: [null], enabled: true, weed: [] },
    ];
    el = document.createElement("div");
    document.body.appendChild(el);
    root = createRoot(el);
    await act(async () => {
      root.render(createElement(CutPreview, { scene, artboard: { x: 0, y: 0, w: 100, h: 100 }, passes, travel: [], theme: "light" }));
    });

    const fills = draws.filter((d) => d.op === "fill").map((d) => d.style);
    expect(fills).toEqual([
      vinylDisplay(0xff0000ff, "light").fill,
      vinylDisplay(0xff0000ff, "light").fill,
      vinylDisplay(0x00ff00ff, "light").fill,
    ]);
    expect(draws.filter((d) => d.op === "fillText").map((d) => d.text)).toEqual(["1"]);
  });
});
