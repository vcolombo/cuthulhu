// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { clippedEdges, contentBounds, fitViewport, type Bounds, type Viewport } from "./viewmodel";

const CANVAS = { w: 400, h: 300 };
const MARGIN = 16;
const ARTBOARD: Bounds = { x: 0, y: 0, w: 330, h: 3000 };

const toScreen = (vp: Viewport, x: number, y: number) => ({
  x: x * vp.scale + vp.tx,
  y: y * vp.scale + vp.ty,
});

describe("contentBounds", () => {
  it("is null for an empty plan", () => {
    expect(contentBounds([], [])).toBeNull();
  });

  it("unions shape bounds with travel endpoints", () => {
    // Both travel endpoints sit outside the shape box, so an implementation that
    // ignores either endpoint produces a smaller union and fails.
    const b = contentBounds([{ x: 10, y: 10, w: 5, h: 5 }], [[0, 0, 30, 40]]);
    expect(b).toEqual({ x: 0, y: 0, w: 30, h: 40 });
  });
});

describe("fitViewport", () => {
  it("fits the artboard when the plan is empty", () => {
    const vp = fitViewport(null, ARTBOARD, CANVAS, MARGIN);
    const top = toScreen(vp, ARTBOARD.x, ARTBOARD.y);
    const bottom = toScreen(vp, ARTBOARD.x + ARTBOARD.w, ARTBOARD.y + ARTBOARD.h);
    expect(vp.scale).toBeGreaterThan(0);
    expect(top.y).toBeCloseTo(MARGIN);
    expect(bottom.y).toBeCloseTo(CANVAS.h - MARGIN);
  });

  it("makes tiny content occupy most of the canvas", () => {
    // A 10×10mm sticker on the Cameo artboard — the case the fixed 1px=1mm
    // mapping rendered as a 10px speck in the corner.
    const vp = fitViewport({ x: 5, y: 5, w: 10, h: 10 }, ARTBOARD, CANVAS, MARGIN);
    const tl = toScreen(vp, 5, 5);
    const br = toScreen(vp, 15, 15);
    expect(br.x - tl.x).toBeCloseTo(CANVAS.h - 2 * MARGIN); // height-limited uniform scale
    expect(tl.x).toBeGreaterThanOrEqual(0);
    expect(br.y).toBeLessThanOrEqual(CANVAS.h);
  });

  it("shrinks content larger than the canvas until fully visible", () => {
    const vp = fitViewport({ x: 0, y: 0, w: 2000, h: 500 }, ARTBOARD, CANVAS, MARGIN);
    expect(vp.scale).toBeLessThan(1);
    const tl = toScreen(vp, 0, 0);
    const br = toScreen(vp, 2000, 500);
    expect(tl.x).toBeCloseTo(MARGIN);
    expect(br.x).toBeCloseTo(CANVAS.w - MARGIN);
    expect(tl.y).toBeGreaterThanOrEqual(0);
    expect(br.y).toBeLessThanOrEqual(CANVAS.h);
  });

  it("centers content offset far from the origin", () => {
    const vp = fitViewport({ x: 300, y: 2900, w: 20, h: 20 }, ARTBOARD, CANVAS, MARGIN);
    const center = toScreen(vp, 310, 2910);
    expect(center.x).toBeCloseTo(CANVAS.w / 2);
    expect(center.y).toBeCloseTo(CANVAS.h / 2);
  });

  it("keeps scale positive and finite when the margin swallows the canvas", () => {
    const vp = fitViewport({ x: 0, y: 0, w: 10, h: 10 }, ARTBOARD, { w: 20, h: 20 }, 50);
    expect(Number.isFinite(vp.scale)).toBe(true);
    expect(vp.scale).toBeGreaterThan(0);
  });

  it("widens degenerate content instead of fitting at Infinity", () => {
    const vp = fitViewport({ x: 50, y: 50, w: 0, h: 0 }, ARTBOARD, CANVAS, MARGIN);
    expect(Number.isFinite(vp.scale)).toBe(true);
    const dot = toScreen(vp, 50, 50);
    expect(dot.x).toBeCloseTo(CANVAS.w / 2);
    expect(dot.y).toBeCloseTo(CANVAS.h / 2);
  });
});

describe("clippedEdges", () => {
  it("reports no clipping when the whole artboard fits", () => {
    const vp = fitViewport(null, ARTBOARD, CANVAS, MARGIN);
    expect(clippedEdges(vp, ARTBOARD, CANVAS)).toEqual({
      left: false, right: false, top: false, bottom: false,
    });
  });

  it("reports the edges a content-fitted view cuts off", () => {
    // Content hugging the artboard's top-left corner (within the fit margin):
    // zooming in keeps the sheet's top-left visible while its right and bottom
    // edges run past the canvas.
    const vp = fitViewport({ x: 0.2, y: 0.2, w: 10, h: 10 }, ARTBOARD, CANVAS, MARGIN);
    expect(clippedEdges(vp, ARTBOARD, CANVAS)).toEqual({
      left: false, right: true, top: false, bottom: true,
    });
  });

  it("reports all edges clipped when content sits mid-sheet", () => {
    const vp = fitViewport({ x: 150, y: 1500, w: 10, h: 10 }, ARTBOARD, CANVAS, MARGIN);
    expect(clippedEdges(vp, ARTBOARD, CANVAS)).toEqual({
      left: true, right: true, top: true, bottom: true,
    });
  });
});
import {
  reorderPass,
  effectiveSettings,
  fieldDisabled,
  toCutRequest,
  type PassVm,
  type Caps,
  type Preset,
} from "./viewmodel";
import type { DeviceInfo } from "../ipc";

const aDevice = (): DeviceInfo => ({
  instance_id: "usb:mock",
  machine_id: "cameo5",
  transport: { Usb: { locator: "mock" } },
  candidate: false,
  host: null,
});

describe("reorderPass", () => {
  it("swaps adjacent passes when within bounds", () => {
    const passes: PassVm[] = [
      {
        color: 0xff0000,
        shapeCount: 5,
        enabled: true,
        presetId: "p1",
        speed: 100,
        force: 50,
        repeatCount: 1,
      },
      {
        color: 0x00ff00,
        shapeCount: 3,
        enabled: true,
        presetId: "p2",
        speed: 120,
        force: 60,
        repeatCount: 1,
      },
    ];

    const result = reorderPass(passes, 0, 1);
    expect(result[0]).toEqual(passes[1]);
    expect(result[1]).toEqual(passes[0]);
  });

  it("clamps at the start (index=0, dir=-1)", () => {
    const passes: PassVm[] = [
      {
        color: 0xff0000,
        shapeCount: 5,
        enabled: true,
        presetId: "p1",
        speed: 100,
        force: 50,
        repeatCount: 1,
      },
      {
        color: 0x00ff00,
        shapeCount: 3,
        enabled: true,
        presetId: "p2",
        speed: 120,
        force: 60,
        repeatCount: 1,
      },
    ];

    const result = reorderPass(passes, 0, -1);
    expect(result).toEqual(passes);
  });

  it("clamps at the end (index=length-1, dir=1)", () => {
    const passes: PassVm[] = [
      {
        color: 0xff0000,
        shapeCount: 5,
        enabled: true,
        presetId: "p1",
        speed: 100,
        force: 50,
        repeatCount: 1,
      },
      {
        color: 0x00ff00,
        shapeCount: 3,
        enabled: true,
        presetId: "p2",
        speed: 120,
        force: 60,
        repeatCount: 1,
      },
    ];

    const result = reorderPass(passes, 1, 1);
    expect(result).toEqual(passes);
  });
});

describe("effectiveSettings", () => {
  it("uses pass override over preset", () => {
    const pass: PassVm = {
      color: 0xff0000,
      shapeCount: 5,
      enabled: true,
      presetId: "preset1",
      speed: 100,
      force: 50,
      repeatCount: 2,
    };

    const result = effectiveSettings(pass, []);
    expect(result.speed).toBe(100);
    expect(result.force).toBe(50);
    expect(result.repeatCount).toBe(2);
  });

  it("falls back to preset when pass fields are null", () => {
    const pass: PassVm = {
      color: 0xff0000,
      shapeCount: 5,
      enabled: true,
      presetId: "preset1",
      speed: null,
      force: null,
      repeatCount: null,
    };

    const presets: Preset[] = [
      {
        id: "preset1",
        name: "Acrylic",
        machine_id: "cameo5",
        settings: { speed: 150, force: 75, repeat_count: 3 },
        builtin: true,
      },
    ];

    const result = effectiveSettings(pass, presets);
    expect(result.speed).toBe(150);
    expect(result.force).toBe(75);
    expect(result.repeatCount).toBe(3);
  });

  it("uses default repeatCount=1 when no preset match and pass is null", () => {
    const pass: PassVm = {
      color: 0xff0000,
      shapeCount: 5,
      enabled: true,
      presetId: null,
      speed: null,
      force: null,
      repeatCount: null,
    };

    const result = effectiveSettings(pass, []);
    expect(result.speed).toBeNull();
    expect(result.force).toBeNull();
    expect(result.repeatCount).toBe(1);
  });

  it("handles partial overrides (speed override, force from preset)", () => {
    const pass: PassVm = {
      color: 0xff0000,
      shapeCount: 5,
      enabled: true,
      presetId: "preset1",
      speed: 100,
      force: null,
      repeatCount: null,
    };

    const presets: Preset[] = [
      {
        id: "preset1",
        name: "Paper",
        machine_id: "cameo5",
        settings: { speed: 150, force: 75, repeat_count: 3 },
        builtin: false,
      },
    ];

    const result = effectiveSettings(pass, presets);
    expect(result.speed).toBe(100);
    expect(result.force).toBe(75);
    expect(result.repeatCount).toBe(3);
  });
});

describe("fieldDisabled", () => {
  it("returns true for speed when supportsSpeed is false", () => {
    const caps: Caps = {
      supportsSpeed: false,
      supportsForce: true,
      needsOperatorPassConfirm: false,
    };

    expect(fieldDisabled("speed", caps)).toBe(true);
  });

  it("returns false for speed when supportsSpeed is true", () => {
    const caps: Caps = {
      supportsSpeed: true,
      supportsForce: true,
      needsOperatorPassConfirm: false,
    };

    expect(fieldDisabled("speed", caps)).toBe(false);
  });

  it("returns true for force when supportsForce is false", () => {
    const caps: Caps = {
      supportsSpeed: true,
      supportsForce: false,
      needsOperatorPassConfirm: false,
    };

    expect(fieldDisabled("force", caps)).toBe(true);
  });

  it("returns false for force when supportsForce is true", () => {
    const caps: Caps = {
      supportsSpeed: true,
      supportsForce: true,
      needsOperatorPassConfirm: false,
    };

    expect(fieldDisabled("force", caps)).toBe(false);
  });
});

describe("toCutRequest", () => {
  it("serializes PassVm to CutRequest with ConfiguredPassDto fields", () => {
    const passes: PassVm[] = [
      {
        color: 0xff0000,
        shapeCount: 5,
        enabled: true,
        presetId: "preset1",
        speed: 100,
        force: 50,
        repeatCount: 2,
      },
    ];

    const result = toCutRequest("device123", "42", passes);

    expect(result.device_instance_id).toBe("device123");
    expect(result.doc_revision).toBe("42");
    expect(result.passes).toHaveLength(1);
    expect(result.passes[0]).toEqual({
      color: 0xff0000,
      enabled: true,
      preset_id: "preset1",
      speed: 100,
      force: 50,
      repeat_count: 2,
    });
  });

  it("serializes null values explicitly in ConfiguredPassDto", () => {
    const passes: PassVm[] = [
      {
        color: null,
        shapeCount: 3,
        enabled: false,
        presetId: null,
        speed: null,
        force: null,
        repeatCount: null,
      },
    ];

    const result = toCutRequest("device123", "42", passes);

    expect(result.passes[0]).toEqual({
      color: null,
      enabled: false,
      preset_id: null,
      speed: null,
      force: null,
      repeat_count: null,
    });
  });

  it("handles multiple passes", () => {
    const passes: PassVm[] = [
      {
        color: 0xff0000,
        shapeCount: 5,
        enabled: true,
        presetId: "preset1",
        speed: 100,
        force: 50,
        repeatCount: 1,
      },
      {
        color: 0x00ff00,
        shapeCount: 3,
        enabled: true,
        presetId: "preset2",
        speed: 120,
        force: 60,
        repeatCount: 2,
      },
    ];

    const result = toCutRequest("device123", "42", passes);

    expect(result.passes).toHaveLength(2);
    expect(result.passes[0].preset_id).toBe("preset1");
    expect(result.passes[1].preset_id).toBe("preset2");
  });
});

describe("DeviceInfo.host", () => {
  it("distinguishes a cutter on this computer from one on a Cut Host", () => {
    const local: DeviceInfo = { ...aDevice(), host: null };
    const remote: DeviceInfo = { ...aDevice(), instance_id: "usb:sn:PI", host: "host-1" };
    expect(local.host).toBeNull();
    expect(remote.host).toBe("host-1");
  });
});
