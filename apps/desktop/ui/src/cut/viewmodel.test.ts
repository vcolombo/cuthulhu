// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import type { DeviceState } from "../ipc";
import {
  reorderPass,
  effectiveSettings,
  fieldDisabled,
  acceptEvent,
  toCutRequest,
  dialogPhase,
  canStartCut,
  dialogButtons,
  type PassVm,
  type Caps,
  type Preset,
} from "./viewmodel";

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

describe("acceptEvent", () => {
  it("accepts event when currentJobId is null", () => {
    expect(acceptEvent(null, { job_id: 1 })).toBe(true);
  });

  it("accepts event when currentJobId matches job_id", () => {
    expect(acceptEvent(5, { job_id: 5 })).toBe(true);
  });

  it("rejects stale event when currentJobId does not match", () => {
    expect(acceptEvent(2, { job_id: 1 })).toBe(false);
  });

  it("rejects stale event with different numbers", () => {
    expect(acceptEvent(10, { job_id: 9 })).toBe(false);
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

  it("omits null values in ConfiguredPassDto", () => {
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

// One state per DeviceState variant (the union has 11 members) so every branch of
// dialogPhase/canStartCut/dialogButtons is exercised at least once.
const STATES: Record<string, DeviceState> = {
  Disconnected: "Disconnected",
  Connecting: "Connecting",
  Idle: "Idle",
  Transmitting: { Transmitting: { job_id: 1, pass_index: 0, submitted_bytes: 10, total_bytes: 100 } },
  AwaitingCompletion: { AwaitingCompletion: { job_id: 1, pass_index: 0 } },
  WaitingForColorSwap: { WaitingForColorSwap: { job_id: 1, next_pass_index: 1 } },
  CancelRequested: { CancelRequested: { job_id: 1 } },
  Stopping: { Stopping: { job_id: 1 } },
  Cancelled: { Cancelled: { job_id: 1, pass_index: 0, submitted_bytes: 10, completion_known: true } },
  Disconnecting: "Disconnecting",
  Error: { Error: "Timeout" },
};

describe("dialogPhase", () => {
  it("maps Disconnected", () => expect(dialogPhase(STATES.Disconnected)).toEqual({ kind: "disconnected" }));
  it("maps Connecting", () => expect(dialogPhase(STATES.Connecting)).toEqual({ kind: "connecting" }));
  it("maps Idle", () => expect(dialogPhase(STATES.Idle)).toEqual({ kind: "idle" }));
  it("maps Transmitting", () => {
    expect(dialogPhase(STATES.Transmitting)).toEqual({ kind: "transmitting", passIndex: 0, submittedBytes: 10, totalBytes: 100 });
  });
  it("maps AwaitingCompletion", () => {
    expect(dialogPhase(STATES.AwaitingCompletion)).toEqual({ kind: "awaitingCompletion", passIndex: 0 });
  });
  it("maps WaitingForColorSwap", () => {
    expect(dialogPhase(STATES.WaitingForColorSwap)).toEqual({ kind: "waitingSwap", nextPassIndex: 1 });
  });
  it("maps CancelRequested", () => expect(dialogPhase(STATES.CancelRequested)).toEqual({ kind: "cancelRequested" }));
  it("maps Stopping", () => expect(dialogPhase(STATES.Stopping)).toEqual({ kind: "stopping" }));
  it("maps Cancelled", () => {
    expect(dialogPhase(STATES.Cancelled)).toEqual({ kind: "cancelled", passIndex: 0, submittedBytes: 10, completionKnown: true });
  });
  it("maps Disconnecting", () => expect(dialogPhase(STATES.Disconnecting)).toEqual({ kind: "disconnecting" }));
  it("maps Error", () => expect(dialogPhase(STATES.Error)).toEqual({ kind: "error" }));
});

describe("canStartCut", () => {
  it("is true for idle and cancelled (mirrors driver-core's Idle | Cancelled precondition)", () => {
    expect(canStartCut(dialogPhase(STATES.Idle))).toBe(true);
    expect(canStartCut(dialogPhase(STATES.Cancelled))).toBe(true);
  });

  it("is false for every other variant", () => {
    for (const key of Object.keys(STATES)) {
      if (key === "Idle" || key === "Cancelled") continue;
      expect(canStartCut(dialogPhase(STATES[key])), key).toBe(false);
    }
  });
});

describe("dialogButtons", () => {
  it("offers only Start (disabled-eligible) for disconnected/connecting/idle/disconnecting/error/cancelled", () => {
    for (const key of ["Disconnected", "Connecting", "Idle", "Disconnecting", "Error", "Cancelled"]) {
      const buttons = dialogButtons(dialogPhase(STATES[key]));
      expect(buttons, key).toEqual({ start: true, resume: false, confirmPassDone: false, cancel: false });
    }
  });

  it("offers Resume + Cancel for WaitingForColorSwap", () => {
    expect(dialogButtons(dialogPhase(STATES.WaitingForColorSwap))).toEqual({
      start: false,
      resume: true,
      confirmPassDone: false,
      cancel: true,
    });
  });

  it("offers Confirm pass done + Cancel for AwaitingCompletion", () => {
    expect(dialogButtons(dialogPhase(STATES.AwaitingCompletion))).toEqual({
      start: false,
      resume: false,
      confirmPassDone: true,
      cancel: true,
    });
  });

  it("offers only Cancel for Transmitting, CancelRequested, and Stopping", () => {
    for (const key of ["Transmitting", "CancelRequested", "Stopping"]) {
      expect(dialogButtons(dialogPhase(STATES[key])), key).toEqual({
        start: false,
        resume: false,
        confirmPassDone: false,
        cancel: true,
      });
    }
  });
});
