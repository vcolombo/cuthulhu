// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import type { CutStatus } from "../../ipc";
import { DISCONNECTED_STATUS } from "../../ipc";
import {
  cancelLabel,
  currentPass,
  failureCopy,
  jobIsActive,
  jobSurface,
  loadPrompt,
  lostBanner,
  lostLabel,
  passPercent,
  percentLabel,
  placeOf,
  waitingLine,
  type JobSnapshot,
} from "./model";

function status(patch: Partial<CutStatus> & Pick<CutStatus, "phase">): CutStatus {
  return {
    ...DISCONNECTED_STATUS,
    ended: null,
    actions: { cut: false, cancel: false, resume: false, confirm: false },
    pass: null,
    sent: null,
    error: null,
    ...patch,
  };
}

const snapshot: JobSnapshot = {
  grouping: "Color",
  passes: [
    { key: "color:ff0000ff", shapeCount: 2 },
    { key: "color:00ff00ff", shapeCount: 1 },
    { key: "color:d39900ff", shapeCount: 3 },
  ],
};

describe("jobSurface", () => {
  it("stays hidden until this session sent a job", () => {
    expect(jobSurface(status({ phase: "Sending", actions: { cut: false, cancel: true, resume: false, confirm: false } }), false)).toBe("hidden");
  });

  it("follows resume and confirm rather than the phase name", () => {
    const swap = status({
      phase: "Idle",
      actions: { cut: false, cancel: true, resume: true, confirm: false },
    });
    expect(jobSurface(swap, true)).toBe("swap");
    const confirm = status({
      phase: "AwaitingConfirmation",
      actions: { cut: false, cancel: true, resume: false, confirm: true },
    });
    expect(jobSurface(confirm, true)).toBe("confirm");
    // A colour-swap phase with resume withheld is not the bar.
    expect(jobSurface(status({ phase: "AwaitingColorSwap" }), true)).toBe("hidden");
  });

  it("does not grow a panel button out of the cut action", () => {
    expect(jobSurface(status({ phase: "Idle", actions: { cut: true, cancel: false, resume: false, confirm: false } }), true)).toBe("hidden");
  });

  it("names the two endings from ended, and a failure from the phase", () => {
    expect(jobSurface(status({ phase: "Idle", ended: "Completed" }), true)).toBe("completed");
    expect(jobSurface(status({ phase: "Idle", ended: "Cancelled" }), true)).toBe("cancelled");
    expect(jobSurface(status({ phase: "Failed", error: "Timeout" }), true)).toBe("failed");
    expect(jobSurface(status({ phase: "Cancelling" }), true)).toBe("cancelling");
    expect(jobSurface(status({ phase: "Sending", actions: { cut: false, cancel: true, resume: false, confirm: false } }), true)).toBe("progress");
  });
});

describe("pass copy", () => {
  it("rounds the worker's byte count and refuses an empty total", () => {
    expect(passPercent(null)).toBeNull();
    expect(passPercent({ sent: 1, total: 0 })).toBeNull();
    expect(passPercent({ sent: 40, total: 100 })).toBe(40);
    expect(percentLabel(40)).toBe("40% of this pass");
  });

  it("names the vinyl the Send sheet would, and indexes the enabled pass", () => {
    expect(currentPass(snapshot, status({ phase: "Sending", pass: { index: 0, total: 3 } }))).toMatchObject({
      name: "Red vinyl",
      swatch: "#ff0000",
      shapeCount: 2,
    });
    expect(currentPass(snapshot, status({ phase: "AwaitingColorSwap", pass: { index: 1, total: 3 } }))).toMatchObject({
      name: "#00ff00",
      swatch: "#00ff00",
    });
    expect(currentPass(snapshot, status({ phase: "AwaitingColorSwap", pass: { index: 2, total: 3 } }))?.name).toBe("Gold vinyl");
    expect(currentPass(snapshot, status({ phase: "Sending", pass: { index: 9, total: 3 } }))).toBeNull();
    expect(loadPrompt("Gold vinyl")).toBe("Load Gold vinyl, then press Resume");
    expect(loadPrompt("#00ff00")).toBe("Load #00ff00, then press Resume");
    expect(waitingLine("Silhouette Cameo 5 Alpha", { index: 1, total: 2 }, 1)).toBe(
      "Silhouette Cameo 5 Alpha is waiting · pass 2 of 2 · 1 shape",
    );
    expect(waitingLine("Cameo", null, null)).toBe("Cameo is waiting");
  });
});

describe("offline and failure copy", () => {
  it("counts real seconds and keeps cancel available under a different label", () => {
    expect(lostLabel(5)).toBe("Connection lost · last update 5s ago");
    expect(lostBanner("Bench Pi")).toBe("Lost connection to Bench Pi, the cut is continuing on the host.");
    expect(cancelLabel("panel", false)).toBe("Cancel");
    expect(cancelLabel("swap", false)).toBe("Cancel job");
    expect(cancelLabel("swap", true)).toBe("Cancel (when reconnected)");
  });

  it("maps each device error and keeps the raw text", () => {
    expect(failureCopy(null)).toEqual({ summary: "The cut failed.", details: "" });
    expect(failureCopy("Timeout")).toEqual({
      summary: "The cutter did not answer in time.",
      details: "Timeout",
    });
    expect(failureCopy("Busy").summary).toBe("The cutter is busy.");
    expect(failureCopy("Disconnected").summary).toBe("The cutter disconnected.");
    expect(failureCopy("WriteZero").summary).toBe("The cutter stopped accepting the job.");
    expect(failureCopy({ Io: "broken pipe" })).toEqual({
      summary: "The cutter could not be written to.",
      details: "broken pipe",
    });
  });

  it("places a local cutter on this computer and a host cutter on the host", () => {
    const machines = [{ id: "cameo5", name: "Silhouette Cameo 5 Alpha" }];
    expect(placeOf({ machine_id: "cameo5", host: null }, machines, [])).toEqual({
      cutter: "Silhouette Cameo 5 Alpha",
      where: "This computer",
    });
    expect(placeOf({ machine_id: "cameo5", host: "host-3" }, machines, [{ id: "host-3", name: "Bench Pi" }])).toEqual({
      cutter: "Silhouette Cameo 5 Alpha",
      where: "Bench Pi",
    });
  });
});

describe("jobIsActive", () => {
  it("matches the worker's active phases", () => {
    expect(jobIsActive("Sending")).toBe(true);
    expect(jobIsActive("AwaitingConfirmation")).toBe(true);
    expect(jobIsActive("AwaitingColorSwap")).toBe(true);
    expect(jobIsActive("Cancelling")).toBe(true);
    expect(jobIsActive("Idle")).toBe(false);
    expect(jobIsActive("Failed")).toBe(false);
    expect(jobIsActive("Disconnected")).toBe(false);
  });
});
