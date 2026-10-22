// SPDX-License-Identifier: GPL-3.0-or-later
import { clampChroma, converter, formatHex, interpolate, wcagContrast, type Oklch } from "culori";

const toOklch = converter("oklch");
const toRgb = converter("rgb");

/** The dark mat the outline rule is measured against. Light outlines are not re-fit to the light mat. */
const DARK_MAT = "#484848";

export type VinylTheme = "light" | "dark";

export type VinylDisplay = {
  fill: string;
  stroke: string;
  swatch: string;
  fillOpacity: number;
};

/** Built-in vinyl colours. Gold is hue 82: hue 70 is amber, and the palette is what was wrong. */
export const VINYL_SWATCHES: { name: string; oklch: string }[] = [
  { name: "Red", oklch: "oklch(0.56 0.2 27)" },
  { name: "Gold", oklch: "oklch(0.72 0.15 82)" },
  { name: "Purple", oklch: "oklch(0.48 0.17 300)" },
  { name: "Orange", oklch: "oklch(0.66 0.17 50)" },
  { name: "Pink", oklch: "oklch(0.62 0.2 350)" },
  { name: "Navy", oklch: "oklch(0.45 0.15 265)" },
  { name: "White", oklch: "oklch(0.95 0 0)" },
  { name: "Black", oklch: "oklch(0.22 0 0)" },
];

export type VinylStyle = { stroke: number | null; fill: number | null };

function visible(rgba: number | null | undefined): number | null {
  if (rgba == null) return null;
  // The low byte is alpha. Zero alpha is not a colour the planner will group.
  if ((rgba & 0xff) === 0) return null;
  return rgba;
}

/** The colour a Colour grouping would key this shape by: visible stroke, else visible fill. */
export function colorModeVinyl(style: VinylStyle | null | undefined): number | null {
  if (!style) return null;
  return visible(style.stroke) ?? visible(style.fill);
}

/**
 * Alpha under 0.1 still cuts (`0xFF000001` in document). 25/255 is just under 0.1 and 26/255 is just over,
 * so the note appears for a byte below 26 and not for a fully transparent channel.
 */
export function nearlyInvisible(rgba: number): boolean {
  const alpha = rgba & 0xff;
  return alpha > 0 && alpha < 26;
}

type WalkNode = { kind: unknown; style?: VinylStyle; children: number[] };

/** True when any selected shape's colour-mode vinyl is nearly invisible in the file. */
export function selectionHasFaintVinyl(nodes: Record<string, WalkNode>, selected: number[]): boolean {
  const seen = new Set<number>();
  const stack = [...selected];
  while (stack.length > 0) {
    const id = stack.pop();
    if (id === undefined || seen.has(id)) continue;
    seen.add(id);
    const node = nodes[String(id)];
    if (!node) continue;
    if (node.kind !== null && typeof node.kind === "object" && "Shape" in node.kind) {
      const vinyl = colorModeVinyl(node.style);
      if (vinyl !== null && nearlyInvisible(vinyl)) return true;
    } else {
      stack.push(...node.children);
    }
  }
  return false;
}

function rgbaToHex(rgba: number): string {
  const r = (rgba >>> 24) & 0xff;
  const g = (rgba >>> 16) & 0xff;
  const b = (rgba >>> 8) & 0xff;
  return `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
}

function passthrough(color: string, theme: VinylTheme, ghostOpacity?: number): VinylDisplay {
  const fillOpacity = ghostOpacity !== undefined && ghostOpacity < 0.4 ? ghostOpacity : theme === "light" ? 0.6 : 0.9;
  return { fill: color, stroke: color, swatch: color, fillOpacity };
}

function gamut(color: Oklch): Oklch {
  return clampChroma(color, "oklch", "rgb") ?? color;
}

/** Maximum sRGB chroma at this lightness. 0.4 is above the gamut for every hue, so the clamp is the maximum. */
function maxChroma(l: number, h: number): Oklch {
  return gamut({ mode: "oklch", l, c: 0.4, h });
}

function compositeOver(fillHex: string, opacity: number, mat: string): string {
  const fill = toRgb(fillHex);
  const ground = toRgb(mat);
  if (!fill || !ground) return fillHex;
  return (
    formatHex({
      mode: "rgb",
      r: fill.r * opacity + ground.r * (1 - opacity),
      g: fill.g * opacity + ground.g * (1 - opacity),
      b: fill.b * opacity + ground.b * (1 - opacity),
    }) ?? fillHex
  );
}

/**
 * How a vinyl colour is drawn. Light output is the approved v6 rendering (fill is the base,
 * outline is `color-mix(in oklab, base 70%, black)` baked to hex because Canvas2D has no color-mix).
 * Dark outlines walk a 0.005 lightness grid and are tested before hex rounding: rounding first
 * stops one step early (navy lands on `#6291ff` instead of `#6592ff`), and a finer step lands
 * on a different red (`#ff5f55` instead of `#ff6056`). The walk starts at the fill, not at the
 * darkest lightness that clears 3:1, so gold stays `#eaab00` rather than dropping to `#c18d00`.
 */
export function vinylDisplay(rgba: number | string, theme: VinylTheme, ghostOpacity?: number): VinylDisplay {
  const css = typeof rgba === "number" ? rgbaToHex(rgba) : rgba.trim();
  const base = toOklch(css);
  if (!base || !Number.isFinite(base.l)) return passthrough(css, theme, ghostOpacity);

  const chroma = base.c ?? 0;
  const hue = base.h ?? 0;
  const veryDark = base.l < 0.38;
  const veryLight = base.l >= 0.88;
  const ghost = ghostOpacity !== undefined && ghostOpacity < 0.4;

  if (theme === "light") {
    const fill = formatHex(base) ?? css;
    // 0.3 toward black is the 70% base of the color-mix the light theme was approved with.
    const stroke = formatHex(interpolate([css, "#000000"], "oklab")(0.3)) ?? fill;
    return { fill, stroke, swatch: fill, fillOpacity: ghost ? ghostOpacity : 0.6 };
  }

  // Hues 60–110 keep chroma when they get light, which is what lets gold reach L 0.78 instead of amber.
  let target = 0.72 - Math.min(chroma, 0.3) * 0.3;
  if (!veryDark && !veryLight && hue >= 60 && hue <= 110) target = Math.max(target, 0.78);
  const fill: Oklch = {
    mode: "oklch",
    l: veryDark || veryLight ? base.l : Math.max(base.l, target),
    c: veryDark || veryLight ? chroma : chroma * 1.04,
    h: hue,
  };
  let filled = gamut(fill);
  // A saturated hue that cannot get this light in sRGB keeps its chroma and gives the lightness back.
  while (!veryDark && !veryLight && filled.c < chroma * 0.9 && fill.l > base.l) {
    fill.l = Math.max(base.l, fill.l - 0.005);
    filled = gamut(fill);
  }

  let outlined: Oklch;
  if (veryLight) {
    outlined = gamut({ mode: "oklch", l: Math.max(0, base.l - 0.07), c: chroma, h: hue });
  } else if (veryDark) {
    let l = Math.max(base.l + 0.06, 0.74);
    outlined = gamut({ mode: "oklch", l, c: chroma, h: hue });
    while (wcagContrast(outlined, DARK_MAT) < 3.05 && l < 0.999) {
      l = Math.min(1, l + 0.005);
      outlined = gamut({ mode: "oklch", l, c: chroma, h: hue });
    }
  } else {
    let l = filled.l;
    outlined = maxChroma(l, hue);
    while (wcagContrast(outlined, DARK_MAT) < 3.05 && l < 0.999) {
      l = Math.min(1, l + 0.005);
      outlined = maxChroma(l, hue);
    }
  }

  const fillHex = formatHex(filled);
  const strokeHex = formatHex(outlined);
  if (!fillHex || !strokeHex) return passthrough(css, theme, ghostOpacity);

  const fillOpacity = ghost ? ghostOpacity : veryDark || veryLight ? 1 : 0.9;
  return {
    fill: fillHex,
    stroke: strokeHex,
    swatch: compositeOver(fillHex, fillOpacity, DARK_MAT),
    fillOpacity,
  };
}
