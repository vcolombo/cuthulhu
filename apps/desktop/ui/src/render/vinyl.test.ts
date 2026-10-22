// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { converter, formatHex, interpolate, wcagContrast, clampChroma } from "culori";
import {
  VINYL_SWATCHES,
  colorModeVinyl,
  nearlyInvisible,
  selectionHasFaintVinyl,
  vinylDisplay,
} from "./vinyl";

const toOklch = converter("oklch");
const DARK_MAT = "#484848";

/** §4.2, recomputed for `#484848`. Hexes are lowercase because that is what the converter emits. */
const DARK_TABLE: { name: string; base: string; fill: string; stroke: string }[] = [
  { name: "red", base: "oklch(0.56 0.2 27)", fill: "#f84d46", stroke: "#ff6056" },
  { name: "gold", base: "oklch(0.72 0.15 82)", fill: "#e9ac19", stroke: "#eaab00" },
  { name: "gold-old", base: "oklch(0.66 0.15 70)", fill: "#f6a32d", stroke: "#fba100" },
  { name: "orange", base: "oklch(0.66 0.17 50)", fill: "#e76d00", stroke: "#f07100" },
  { name: "pink", base: "oklch(0.62 0.2 350)", fill: "#e94da4", stroke: "#ff4fb2" },
  { name: "purple", base: "oklch(0.48 0.17 300)", fill: "#a777f0", stroke: "#b07bff" },
  { name: "purple-test", base: "#7b3fa0", fill: "#b576e0", stroke: "#c76eff" },
  { name: "navy", base: "oklch(0.45 0.15 265)", fill: "#6792f6", stroke: "#6592ff" },
  { name: "navy-test", base: "#113663", fill: "#113663", stroke: "#85aee3" },
  { name: "black", base: "oklch(0.22 0 0)", fill: "#1b1b1b", stroke: "#ababab" },
  { name: "white", base: "oklch(0.95 0 0)", fill: "#eeeeee", stroke: "#d7d7d7" },
  { name: "pale", base: "#f5e6a3", fill: "#f5e6a3", stroke: "#decf8d" },
];

const TEST_COLOURS = ["#1c1c1c", "#ffffff", "#113663", "#7b3fa0", "#f5e6a3"];

function hueDelta(a: number, b: number): number {
  const d = Math.abs(a - b) % 360;
  return Math.min(d, 360 - d);
}

describe("vinylDisplay", () => {
  it("matches the dark contrast table, and every rounded outline clears 3:1 on #484848", () => {
    for (const row of DARK_TABLE) {
      const got = vinylDisplay(row.base, "dark");
      expect(got.fill, row.name).toBe(row.fill);
      expect(got.stroke, row.name).toBe(row.stroke);
      expect(wcagContrast(got.stroke, DARK_MAT), row.name).toBeGreaterThanOrEqual(3);
    }
  });

  it("keeps fill and outline hue within 4° and outline chroma near the gamut edge", () => {
    for (const row of DARK_TABLE) {
      const base = toOklch(row.base);
      const fill = toOklch(vinylDisplay(row.base, "dark").fill);
      const stroke = toOklch(vinylDisplay(row.base, "dark").stroke);
      expect(base && fill && stroke, row.name).toBeTruthy();
      if (!base || !fill || !stroke || (base.c ?? 0) < 1e-3) continue;
      const baseHue = base.h ?? 0;
      expect(hueDelta(fill.h ?? 0, baseHue), row.name).toBeLessThanOrEqual(4);
      expect(hueDelta(stroke.h ?? 0, baseHue), row.name).toBeLessThanOrEqual(4);
    }
    // Very dark and very light outlines keep the base chroma on purpose. The 0.85 rule is the
    // normal case, where the outline is taken at maximum in-gamut chroma.
    for (const swatch of VINYL_SWATCHES) {
      const base = toOklch(swatch.oklch);
      if (!base || base.l < 0.38 || base.l >= 0.88) continue;
      const stroke = toOklch(vinylDisplay(swatch.oklch, "dark").stroke);
      expect(stroke, swatch.name).toBeTruthy();
      if (!stroke) continue;
      const max = clampChroma({ mode: "oklch", l: stroke.l, c: 0.4, h: stroke.h ?? 0 }, "oklch", "rgb");
      const cap = max?.c ?? 0;
      expect(stroke.c, swatch.name).toBeGreaterThanOrEqual(cap * 0.85 - 1e-3);
    }
  });

  it("draws the dark swatch as the fill composited over #484848", () => {
    for (const row of DARK_TABLE) {
      const got = vinylDisplay(row.base, "dark");
      const fill = converterRgb(got.fill);
      const mat = converterRgb(DARK_MAT);
      const swatch = formatHex({
        mode: "rgb",
        r: fill.r * got.fillOpacity + mat.r * (1 - got.fillOpacity),
        g: fill.g * got.fillOpacity + mat.g * (1 - got.fillOpacity),
        b: fill.b * got.fillOpacity + mat.b * (1 - got.fillOpacity),
      });
      expect(got.swatch, row.name).toBe(swatch);
    }
  });

  it("leaves the light theme on the approved mix, including the red outline and the new gold", () => {
    const red = vinylDisplay("oklch(0.56 0.2 27)", "light");
    expect(red.fill).toBe(formatHex("oklch(0.56 0.2 27)"));
    expect(red.stroke).toBe("#801716");
    expect(red.swatch).toBe(red.fill);
    expect(red.fillOpacity).toBe(0.6);

    const gold = vinylDisplay("oklch(0.72 0.15 82)", "light");
    expect(gold.fill).toBe("#d39900");
    expect(gold.swatch).toBe(gold.fill);

    for (const color of [...VINYL_SWATCHES.map((s) => s.oklch), ...TEST_COLOURS]) {
      const got = vinylDisplay(color, "light");
      const mix = formatHex(interpolate([color, "#000000"], "oklab")(0.3));
      expect(got.stroke, color).toBe(mix);
      expect(got.swatch, color).toBe(got.fill);
      expect(got.fillOpacity, color).toBe(0.6);
    }
  });

  it("pins the decided red and gold, and gold's preset hue", () => {
    expect(vinylDisplay("oklch(0.56 0.2 27)", "dark")).toMatchObject({ fill: "#f84d46", stroke: "#ff6056" });
    expect(vinylDisplay("oklch(0.72 0.15 82)", "dark")).toMatchObject({ fill: "#e9ac19", stroke: "#eaab00" });
    expect(VINYL_SWATCHES.find((s) => s.name === "Gold")?.oklch).toBe("oklch(0.72 0.15 82)");
  });

  it("ignores alpha, repeats itself, and passes a CSS variable through", () => {
    expect(vinylDisplay(0xff000001, "dark")).toEqual(vinylDisplay(0xff0000ff, "dark"));
    expect(vinylDisplay(0x7b3fa0ff, "light")).toEqual(vinylDisplay("#7b3fa0", "light"));
    expect(vinylDisplay(0x7b3fa0ff, "dark")).toEqual(vinylDisplay(0x7b3fa0ff, "dark"));
    const passed = vinylDisplay("var(--muted-foreground)", "dark");
    expect(passed.fill).toBe("var(--muted-foreground)");
    expect(passed.stroke).toBe("var(--muted-foreground)");
  });

  it("uses the caller's opacity when a pass is ghosted", () => {
    const solid = vinylDisplay("#7b3fa0", "dark");
    const ghost = vinylDisplay("#7b3fa0", "dark", 0.03);
    expect(ghost.fill).toBe(solid.fill);
    expect(ghost.stroke).toBe(solid.stroke);
    expect(ghost.fillOpacity).toBe(0.03);
    expect(vinylDisplay("#7b3fa0", "light", 0.2).fillOpacity).toBe(0.2);
    expect(vinylDisplay("#7b3fa0", "dark", 0.5).fillOpacity).toBe(solid.fillOpacity);
  });

  it("keeps very-light white and very-dark #1c1c1c on their own fills", () => {
    const white = vinylDisplay("#ffffff", "dark");
    expect(white.fill).toBe("#ffffff");
    expect(white.fillOpacity).toBe(1);
    expect(wcagContrast(white.stroke, DARK_MAT)).toBeGreaterThanOrEqual(3);
    const black = vinylDisplay("#1c1c1c", "dark");
    expect(black.fill).toBe("#1c1c1c");
    expect(black.fillOpacity).toBe(1);
    expect(wcagContrast(black.stroke, DARK_MAT)).toBeGreaterThanOrEqual(3);
  });
});

describe("colour-mode vinyl", () => {
  it("prefers a visible stroke and skips a zero-alpha channel", () => {
    expect(colorModeVinyl({ stroke: 0xff0000ff, fill: 0x00ff00ff })).toBe(0xff0000ff);
    expect(colorModeVinyl({ stroke: 0xff000000, fill: 0x00ff00ff })).toBe(0x00ff00ff);
    expect(colorModeVinyl({ stroke: null, fill: null })).toBeNull();
    expect(colorModeVinyl(undefined)).toBeNull();
  });

  it("calls a byte below 26 nearly invisible and still a colour", () => {
    expect(nearlyInvisible(0xff000001)).toBe(true);
    expect(nearlyInvisible(0xff000019)).toBe(true);
    expect(nearlyInvisible(0xff00001a)).toBe(false);
    expect(nearlyInvisible(0xff000000)).toBe(false);
  });

  it("notices a faint shape inside a selected group", () => {
    const nodes = {
      1: { kind: "Group", style: { stroke: null, fill: null }, children: [2] },
      2: { kind: { Shape: { Rect: { w: 1, h: 1 } } }, style: { stroke: 0xff000001, fill: null }, children: [] },
    };
    expect(selectionHasFaintVinyl(nodes, [1])).toBe(true);
    expect(selectionHasFaintVinyl(nodes, [2])).toBe(true);
    expect(selectionHasFaintVinyl({ 2: { ...nodes[2], style: { stroke: 0xff0000ff, fill: null } } }, [2])).toBe(false);
  });
});

function converterRgb(hex: string): { r: number; g: number; b: number } {
  const rgb = converter("rgb")(hex);
  if (!rgb) throw new Error(hex);
  return rgb;
}
