// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { acceptError, acceptResult, controlsFromSpecs, makeDebouncer, svgDataUrl } from "./viewmodel";

const ready = { svg: "<svg/>", pathCount: 3, downscaled: false };

const specs = {
  controls: [
    { name: "speckle" as const, label: "Ignore speckles", help: "", min: 0, max: 16, step: 1, default: 4, colorOnly: false },
    { name: "smoothing" as const, label: "Smoothing", help: "", min: 0, max: 180, step: 1, default: 60, colorOnly: false },
    { name: "detail" as const, label: "Detail", help: "", min: 3.5, max: 10, step: 0.5, default: 9.5, colorOnly: false },
    { name: "colors" as const, label: "Colors", help: "", min: 1, max: 8, step: 1, default: 6, colorOnly: true },
  ],
  defaultMode: "binary" as const,
  maxDim: 2048,
};

describe("controlsFromSpecs", () => {
  it("starts every control at the default the backend stated", () => {
    expect(controlsFromSpecs(specs)).toEqual({
      mode: "binary", speckle: 4, smoothing: 60, detail: 9.5, colors: 6,
    });
  });
  it("refuses a table missing a control rather than inventing one", () => {
    const short = { ...specs, controls: specs.controls.filter((c) => c.name !== "detail") };
    expect(() => controlsFromSpecs(short)).toThrow(/detail/);
  });
});

describe("staleness", () => {
  it("accepts the latest response", () => {
    expect(acceptResult(2, 2, ready, { kind: "tracing" })).toEqual({ kind: "ready", ...ready });
  });
  it("discards stale results and errors, keeping previous state", () => {
    const prev = { kind: "ready" as const, ...ready };
    expect(acceptResult(1, 2, { ...ready, pathCount: 99 }, prev)).toBe(prev);
    expect(acceptError(1, 2, "trace", "boom", prev)).toBe(prev);
  });
  it("maps the empty code to the empty state, other codes to error", () => {
    expect(acceptError(2, 2, "empty", "nothing traced — lower the speckle filter or raise detail", { kind: "tracing" }))
      .toEqual({ kind: "empty", message: "nothing traced — lower the speckle filter or raise detail" });
    expect(acceptError(2, 2, "trace", "trace failed: x", { kind: "tracing" }))
      .toEqual({ kind: "error", message: "trace failed: x" });
  });
  // Models the dialog's ordering contract: a control change bumps the id up front, so a
  // response for the superseded request is rejected even though its own trace is still running.
  // Bumping only when the debounce fires would let this land and enable Insert on stale geometry.
  it("rejects an in-flight response once a newer request has been issued", () => {
    const inFlightId = 1;
    const afterControlChange = 2; // id bumped by the control change, before any new trace fires
    const prev = { kind: "tracing" as const };
    expect(acceptResult(inFlightId, afterControlChange, ready, prev)).toBe(prev);
  });
});

describe("svgDataUrl", () => {
  it("URI-encodes the svg", () => {
    expect(svgDataUrl('<svg a="b"/>')).toBe("data:image/svg+xml;utf8," + encodeURIComponent('<svg a="b"/>'));
  });
});

describe("makeDebouncer", () => {
  it("collapses rapid calls into the last one", () => {
    vi.useFakeTimers();
    const d = makeDebouncer(300);
    const calls: string[] = [];
    d.schedule(() => calls.push("a"));
    d.schedule(() => calls.push("b"));
    vi.advanceTimersByTime(299);
    expect(calls).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(calls).toEqual(["b"]);
    vi.useRealTimers();
  });
  it("cancel drops the pending call", () => {
    vi.useFakeTimers();
    const d = makeDebouncer(300);
    const calls: string[] = [];
    d.schedule(() => calls.push("a"));
    d.cancel();
    vi.advanceTimersByTime(1000);
    expect(calls).toEqual([]);
    vi.useRealTimers();
  });
});
