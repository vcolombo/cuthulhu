// SPDX-License-Identifier: GPL-3.0-or-later
import type { ControlSpec, TraceControlsDto, TraceControlSpecsDto } from "../ipc";

/// Start every control at the default the backend stated, so no default is restated here.
export function controlsFromSpecs(specs: TraceControlSpecsDto): TraceControlsDto {
  const value = (name: ControlSpec["name"]): number => {
    const spec = specs.controls.find((c) => c.name === name);
    // A missing control means the dialog and the tracer disagree about what a trace takes.
    // Rendering a slider from an invented default would hide that; failing says it.
    if (!spec) throw new Error(`trace_controls omitted "${name}"`);
    return spec.default;
  };
  return {
    mode: specs.defaultMode,
    speckle: value("speckle"),
    smoothing: value("smoothing"),
    detail: value("detail"),
    colors: value("colors"),
  };
}

export type PreviewState =
  | { kind: "idle" }
  | { kind: "tracing" }
  | { kind: "ready"; svg: string; pathCount: number; downscaled: boolean }
  | { kind: "empty"; message: string }
  | { kind: "error"; message: string };

export function acceptResult(
  requestId: number, latestId: number,
  r: { svg: string; pathCount: number; downscaled: boolean },
  prev: PreviewState,
): PreviewState {
  if (requestId !== latestId) return prev;
  return { kind: "ready", svg: r.svg, pathCount: r.pathCount, downscaled: r.downscaled };
}

export function acceptError(
  requestId: number, latestId: number, code: string | null, message: string, prev: PreviewState,
): PreviewState {
  if (requestId !== latestId) return prev;
  // The empty state is a distinct rendering, not an error banner. Selecting it by code rather than
  // by matching the message means the wording can change without breaking the branch.
  return code === "empty" ? { kind: "empty", message } : { kind: "error", message };
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
