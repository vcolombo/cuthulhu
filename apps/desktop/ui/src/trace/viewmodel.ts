// SPDX-License-Identifier: GPL-3.0-or-later

export type TraceControls = {
  mode: "binary" | "color";
  speckle: number;   // 0–16
  smoothing: number; // 0–180
  detail: number;    // 3.5–10, user-facing: higher = more detail
  colors: number;    // 1–8
};

export const defaultControls: TraceControls = { mode: "binary", speckle: 4, smoothing: 60, detail: 9.5, colors: 6 };

export function toOptionsDto(c: TraceControls) {
  // vtracer's length_threshold is inverse to detail (lower threshold = more detail),
  // so flip the user-facing slider value onto the 3.5–10 threshold range.
  return { mode: c.mode, filterSpeckle: c.speckle, cornerThreshold: c.smoothing, lengthThreshold: 13.5 - c.detail, colorPrecision: c.colors };
}

export type PreviewState =
  | { kind: "idle" }
  | { kind: "tracing" }
  | { kind: "ready"; svg: string; pathCount: number; downscaled: boolean }
  | { kind: "empty" }
  | { kind: "error"; message: string };

export function acceptResult(
  requestId: number, latestId: number,
  r: { svg: string; pathCount: number; downscaled: boolean },
  prev: PreviewState,
): PreviewState {
  if (requestId !== latestId) return prev;
  return { kind: "ready", svg: r.svg, pathCount: r.pathCount, downscaled: r.downscaled };
}

export function acceptError(requestId: number, latestId: number, message: string, prev: PreviewState): PreviewState {
  if (requestId !== latestId) return prev;
  return message === "empty" ? { kind: "empty" } : { kind: "error", message };
}

export function svgDataUrl(svg: string): string {
  return "data:image/svg+xml;utf8," + encodeURIComponent(svg);
}

export function makeDebouncer(ms: number) {
  let t: ReturnType<typeof setTimeout> | null = null;
  return {
    schedule(fn: () => void) {
      if (t !== null) clearTimeout(t);
      t = setTimeout(fn, ms);
    },
    cancel() {
      if (t !== null) clearTimeout(t);
      t = null;
    },
  };
}
