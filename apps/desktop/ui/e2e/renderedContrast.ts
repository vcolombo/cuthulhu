// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * Contrast of painted pixels. The page decodes the screenshot; this only
 * scores the RGBA samples. A 12px label is mostly antialiasing, so the ink
 * is the 98th percentile of contrast against the background, not the fringe
 * and not a one-pixel speck.
 */

export type Rgb = [number, number, number];

export type RenderedContrast = {
  ratio: number;
  background: Rgb;
  ink: Rgb;
  inkSamples: number;
  p90: number;
  peak: number;
};

function linear(channel: number): number {
  const s = channel / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}

function luminance(rgb: Rgb): number {
  return 0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2]);
}

export function contrastRatio(a: Rgb, b: Rgb): number {
  const left = luminance(a);
  const right = luminance(b);
  const [hi, lo] = left > right ? [left, right] : [right, left];
  return (hi + 0.05) / (lo + 0.05);
}

/** The flat colour of a crop. A label's fringe is not the fill. */
export function modeRgb(rgba: ArrayLike<number>): Rgb {
  const counts = new Map<string, { n: number; rgb: Rgb }>();
  for (let i = 0; i + 3 < rgba.length; i += 4) {
    if (rgba[i + 3] < 128) continue;
    const rgb: Rgb = [rgba[i], rgba[i + 1], rgba[i + 2]];
    const key = rgb.join(",");
    const seen = counts.get(key);
    if (seen) seen.n += 1;
    else counts.set(key, { n: 1, rgb });
  }
  let background: { n: number; rgb: Rgb } | null = null;
  for (const entry of counts.values()) {
    if (!background || entry.n > background.n) background = entry;
  }
  if (!background) throw new Error("screenshot has no pixels");
  return background.rgb;
}

/**
 * A pressed pill is not the same colour as the surface it sits on. The fill
 * clearing 1.2:1 is enough. A quieter fill still counts when its edge clears
 * 1.2:1 against both the container and the fill, which is a border you can
 * see. Text contrast is a separate check: the edge does not carry the label.
 */
export function pillApart(fill: Rgb, container: Rgb, edge: Rgb): {
  fillRatio: number;
  edgeRatio: number;
  edgeAgainstFill: number;
  apart: boolean;
} {
  const fillRatio = contrastRatio(fill, container);
  const edgeRatio = contrastRatio(edge, container);
  const edgeAgainstFill = contrastRatio(edge, fill);
  const border = edgeRatio >= 1.2 && edgeAgainstFill >= 1.2;
  return { fillRatio, edgeRatio, edgeAgainstFill, apart: fillRatio >= 1.2 || border };
}

export function renderedContrastFromPixels(rgba: ArrayLike<number>): RenderedContrast {
  const counts = new Map<string, { n: number; rgb: Rgb }>();
  const pixels: Rgb[] = [];
  for (let i = 0; i + 3 < rgba.length; i += 4) {
    if (rgba[i + 3] < 128) continue;
    const rgb: Rgb = [rgba[i], rgba[i + 1], rgba[i + 2]];
    pixels.push(rgb);
    const key = rgb.join(",");
    const seen = counts.get(key);
    if (seen) seen.n += 1;
    else counts.set(key, { n: 1, rgb });
  }
  let background: { n: number; rgb: Rgb } | null = null;
  for (const entry of counts.values()) {
    if (!background || entry.n > background.n) background = entry;
  }
  if (!background || pixels.length === 0) throw new Error("screenshot has no pixels");
  const glyph = pixels
    .map((rgb) => ({ rgb, ratio: contrastRatio(rgb, background.rgb) }))
    .filter((pixel) => pixel.ratio > 1.15)
    .sort((a, b) => a.ratio - b.ratio);
  if (glyph.length < 8) throw new Error(`not enough glyph pixels (${glyph.length})`);
  const at = (p: number) => glyph[Math.min(glyph.length - 1, Math.floor(glyph.length * p))];
  const ink = at(0.98);
  return {
    ratio: ink.ratio,
    background: background.rgb,
    ink: ink.rgb,
    inkSamples: glyph.length,
    p90: at(0.9).ratio,
    peak: at(1).ratio,
  };
}
