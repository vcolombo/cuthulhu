// SPDX-License-Identifier: GPL-3.0-or-later

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
  doc_revision: number;
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
export function reorderPass(
  passes: PassVm[],
  index: number,
  dir: -1 | 1
): PassVm[] {
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
  if (field === "force") return !caps.supportsForce;
  return false;
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
  docRevision: number,
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
