// SPDX-License-Identifier: GPL-3.0-or-later
import type { DeviceState } from "../ipc";

// View model types (UI representation)
export type PassVm = {
  color: number | null;
  shapeCount: number;
  enabled: boolean;
  presetId: string | null;
  speed: number | null;
  force: number | null;
  repeatCount: number | null;
};

export type Caps = {
  supportsSpeed: boolean;
  supportsForce: boolean;
  needsOperatorPassConfirm: boolean;
};

// Wire types (match Rust ConfiguredPassDto and CutRequest)
export type ConfiguredPassDto = {
  color: number | null;
  enabled: boolean;
  preset_id: string | null;
  speed: number | null;
  force: number | null;
  repeat_count: number | null;
};

export type CutRequest = {
  device_instance_id: string;
  doc_revision: string;
  passes: ConfiguredPassDto[];
};

// Preset type (mirrors cutplan::MaterialPreset)
export type Preset = {
  id: string;
  name: string;
  machine_id: string;
  settings: {
    speed: number | null;
    force: number | null;
    repeat_count: number;
  };
  builtin: boolean;
};

/**
 * Reorder a pass within the list by swapping it with an adjacent element.
 * Clamps at the start and end bounds.
 *
 * @param passes The list of passes
 * @param index The index of the pass to move
 * @param dir Direction: -1 (up) or 1 (down)
 * @returns A new array with the reordered passes
 */
export function reorderPass<T>(
  passes: T[],
  index: number,
  dir: -1 | 1
): T[] {
  // Clamp: if at boundary in the direction of movement, return unchanged
  if (dir === -1 && index === 0) return passes;
  if (dir === 1 && index === passes.length - 1) return passes;

  const newIndex = index + dir;
  const result = [...passes];
  [result[index], result[newIndex]] = [result[newIndex], result[index]];

  return result;
}

/**
 * Compute effective settings for a pass, accounting for overrides and presets.
 * Priority: pass override > preset > default (repeatCount defaults to 1)
 *
 * @param p The pass
 * @param presets Available presets
 * @returns Effective settings with resolved speed, force, and repeatCount
 */
export function effectiveSettings(
  p: PassVm,
  presets: Preset[]
): {
  speed: number | null;
  force: number | null;
  repeatCount: number;
} {
  // Find matching preset
  const preset = p.presetId ? presets.find((pr) => pr.id === p.presetId) : null;

  // Speed: pass override > preset > null
  const speed =
    p.speed !== null ? p.speed : preset?.settings.speed ?? null;

  // Force: pass override > preset > null
  const force =
    p.force !== null ? p.force : preset?.settings.force ?? null;

  // RepeatCount: pass override > preset > 1
  const repeatCount =
    p.repeatCount !== null
      ? p.repeatCount
      : preset?.settings.repeat_count ?? 1;

  return { speed, force, repeatCount };
}

/**
 * Determine if a field is disabled based on device capabilities.
 *
 * @param field The field name ("speed" or "force")
 * @param caps Device capabilities
 * @returns true if the field should be disabled
 */
export function fieldDisabled(
  field: "speed" | "force",
  caps: Caps
): boolean {
  if (field === "speed") return !caps.supportsSpeed;
  return !caps.supportsForce;
}

/**
 * Filter stale events based on current job ID.
 * Accepts event if no current job exists or if the event matches the current job.
 *
 * @param currentJobId The current job ID, or null if no job is active
 * @param ev Event with job_id field
 * @returns true if the event should be accepted
 */
export function acceptEvent(
  currentJobId: number | null,
  ev: { job_id: number }
): boolean {
  if (currentJobId === null) return true;
  return currentJobId === ev.job_id;
}

/**
 * What a device event means for the current job's lifecycle: whether it latches a
 * completion/failure banner outcome, and whether it ends the job — releasing the
 * job-id event filter (acceptEvent). The filter must be released the moment the
 * job is over, or NO_JOB=0 lifecycle events (e.g. a reconnect after a failed
 * resume) stay filtered for the rest of the session. The banner outcome is
 * latched as separate state precisely so it can outlive that release.
 *
 * Cancelled arrives as a resting *state* (StateChanged), not a terminal event
 * kind, so it releases the filter without latching an outcome — the Cancelled
 * state itself is what the dialog displays.
 */
export function terminalTransition(
  currentJobId: number | null,
  ev: { job_id: number; kind: unknown }
): { outcome: "complete" | "failed" | null; releaseJob: boolean } {
  if (currentJobId === null || ev.job_id !== currentJobId) {
    return { outcome: null, releaseJob: false };
  }
  const k = ev.kind;
  if (k === "JobComplete") return { outcome: "complete", releaseJob: true };
  if (typeof k === "object" && k !== null && "Failed" in k) {
    return { outcome: "failed", releaseJob: true };
  }
  if (typeof k === "object" && k !== null && "StateChanged" in k) {
    const s = (k as { StateChanged: unknown }).StateChanged;
    if (typeof s === "object" && s !== null && "Cancelled" in s) {
      return { outcome: null, releaseJob: true };
    }
  }
  return { outcome: null, releaseJob: false };
}

/**
 * Convert PassVm[] to CutRequest for transmission to Rust backend.
 * Maps camelCase PassVm to snake_case ConfiguredPassDto fields.
 *
 * @param deviceInstanceId Device instance ID
 * @param docRevision Document revision
 * @param passes Array of passes
 * @returns CutRequest ready to send to backend
 */
export function toCutRequest(
  deviceInstanceId: string,
  docRevision: string,
  passes: PassVm[]
): CutRequest {
  return {
    device_instance_id: deviceInstanceId,
    doc_revision: docRevision,
    passes: passes.map((p) => ({
      color: p.color,
      enabled: p.enabled,
      preset_id: p.presetId,
      speed: p.speed,
      force: p.force,
      repeat_count: p.repeatCount,
    })),
  };
}

/**
 * Classified, dialog-relevant view of a raw DeviceState. Every DeviceState
 * variant maps to exactly one DevicePhase — this is the single place that
 * decides what a raw wire state "means" for the cut dialog, so CutDialog.tsx
 * never has to re-derive it ad hoc (and so every variant, including
 * Cancelled, has a tested mapping).
 */
export type DevicePhase =
  | { kind: "disconnected" }
  | { kind: "connecting" }
  | { kind: "idle" }
  | { kind: "disconnecting" }
  | { kind: "transmitting"; passIndex: number; submittedBytes: number; totalBytes: number }
  | { kind: "awaitingCompletion"; passIndex: number }
  | { kind: "waitingSwap"; nextPassIndex: number }
  | { kind: "cancelRequested" }
  | { kind: "stopping" }
  | { kind: "cancelled"; passIndex: number; submittedBytes: number; completionKnown: boolean }
  | { kind: "error" };

export function dialogPhase(state: DeviceState): DevicePhase {
  if (state === "Disconnected") return { kind: "disconnected" };
  if (state === "Connecting") return { kind: "connecting" };
  if (state === "Idle") return { kind: "idle" };
  if (state === "Disconnecting") return { kind: "disconnecting" };
  if ("Transmitting" in state) {
    const t = state.Transmitting;
    return { kind: "transmitting", passIndex: t.pass_index, submittedBytes: t.submitted_bytes, totalBytes: t.total_bytes };
  }
  if ("AwaitingCompletion" in state) return { kind: "awaitingCompletion", passIndex: state.AwaitingCompletion.pass_index };
  if ("WaitingForColorSwap" in state) return { kind: "waitingSwap", nextPassIndex: state.WaitingForColorSwap.next_pass_index };
  if ("CancelRequested" in state) return { kind: "cancelRequested" };
  if ("Stopping" in state) return { kind: "stopping" };
  if ("Cancelled" in state) {
    const c = state.Cancelled;
    return { kind: "cancelled", passIndex: c.pass_index, submittedBytes: c.submitted_bytes, completionKnown: c.completion_known };
  }
  return { kind: "error" };
}

/**
 * Whether Cut can be started from this phase. Mirrors driver-core::manager's
 * `matches!(state, DeviceState::Idle | DeviceState::Cancelled { .. })` — Cancelled
 * is a valid resting state on the Rust side too, not a dead end.
 */
export function canStartCut(phase: DevicePhase): boolean {
  return phase.kind === "idle" || phase.kind === "cancelled";
}

export type DialogButtons = {
  start: boolean;
  resume: boolean;
  confirmPassDone: boolean;
  cancel: boolean;
};

/**
 * Which action buttons apply for a given phase. `cancel` is available in
 * every state that has an active job (including WaitingForColorSwap and
 * AwaitingCompletion) — DeviceManager::cancel() explicitly unparks a worker
 * sitting in either of those states, it isn't limited to mid-transmit.
 */
export function dialogButtons(phase: DevicePhase): DialogButtons {
  const resume = phase.kind === "waitingSwap";
  const confirmPassDone = phase.kind === "awaitingCompletion";
  const cancel =
    phase.kind === "transmitting" ||
    phase.kind === "cancelRequested" ||
    phase.kind === "stopping" ||
    phase.kind === "waitingSwap" ||
    phase.kind === "awaitingCompletion";
  // Start is the default fallback button — it renders whenever no other action
  // button applies (including disconnected/connecting/disconnecting/error), even
  // though it's only enabled (canStartCut) from idle/cancelled. Its disabled state
  // is driven separately by canStartCut() plus dialog-local checks (connected,
  // machine match, row count) that live in CutDialog, not here.
  const start = !resume && !confirmPassDone && !cancel;
  return { start, resume, confirmPassDone, cancel };
}
