// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { acceptError, acceptResult, defaultControls, makeDebouncer, svgDataUrl, toOptionsDto } from "./viewmodel";

const ready = { svg: "<svg/>", pathCount: 3, downscaled: false };

describe("toOptionsDto", () => {
  it("maps control names to the IPC payload", () => {
    expect(toOptionsDto(defaultControls)).toEqual({
      mode: "binary", filterSpeckle: 4, cornerThreshold: 60, lengthThreshold: 4, colorPrecision: 6,
    });
  });
  it("inverts detail onto vtracer's length_threshold", () => {
    expect(toOptionsDto({ ...defaultControls, detail: 3.5 }).lengthThreshold).toBe(10);
  });
});

describe("staleness", () => {
  it("accepts the latest response", () => {
    expect(acceptResult(2, 2, ready, { kind: "tracing" })).toEqual({ kind: "ready", ...ready });
  });
  it("discards stale results and errors, keeping previous state", () => {
    const prev = { kind: "ready" as const, ...ready };
    expect(acceptResult(1, 2, { ...ready, pathCount: 99 }, prev)).toBe(prev);
    expect(acceptError(1, 2, "boom", prev)).toBe(prev);
  });
  it("maps the empty sentinel to the empty state, other messages to error", () => {
    expect(acceptError(2, 2, "empty", { kind: "tracing" })).toEqual({ kind: "empty" });
    expect(acceptError(2, 2, "trace failed: x", { kind: "tracing" })).toEqual({ kind: "error", message: "trace failed: x" });
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
