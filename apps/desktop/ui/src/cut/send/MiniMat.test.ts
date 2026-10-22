// SPDX-License-Identifier: GPL-3.0-or-later
// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { MiniMat, miniMatFrame } from "./MiniMat";
import type { PassRow } from "./useCutPlan";

const ART = { x: 0, y: 0, w: 330, h: 3000 };

describe("miniMatFrame", () => {
  it("keeps a margin when the design sits on the mat's edge", () => {
    const frame = miniMatFrame([{ x: 0, y: 0, w: 10, h: 10 }, { x: 30, y: 0, w: 10, h: 10 }], ART);
    expect(frame.x).toBeLessThan(0);
    expect(frame.y).toBeLessThan(0);
    expect(frame.x + frame.w).toBeGreaterThan(40);
    expect(frame.y + frame.h).toBeGreaterThan(10);
  });

  it("frames the whole mat when nothing is drawn", () => {
    const frame = miniMatFrame([], ART);
    expect(frame.x).toBeLessThan(ART.x);
    expect(frame.y).toBeLessThan(ART.y);
    expect(frame.x + frame.w).toBeGreaterThan(ART.x + ART.w);
    expect(frame.y + frame.h).toBeGreaterThan(ART.y + ART.h);
  });
});

describe("MiniMat", () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  it("draws that frame, so a shape at the origin is not the edge of the picture", async () => {
    const el = document.createElement("div");
    document.body.appendChild(el);
    const root = createRoot(el);
    const row: PassRow = {
      key: "color:ff0000ff",
      shapeCount: 1,
      enabled: true,
      presetId: null,
      speed: null,
      force: null,
      repeatCount: null,
      nodeIds: [1],
      starts: [null],
      weed: [],
    };
    const bounds = { x: 0, y: 0, w: 10, h: 10 };
    await act(async () => {
      root.render(createElement(MiniMat, {
        scene: { nodes: [{ id: 1, bounds }] },
        artboard: ART,
        passes: [row],
        highlight: 0,
        theme: "light",
      }));
    });
    const frame = miniMatFrame([bounds], ART);
    expect(el.querySelector("svg")?.getAttribute("viewBox")).toBe(`${frame.x} ${frame.y} ${frame.w} ${frame.h}`);
    act(() => root.unmount());
    el.remove();
  });
});
