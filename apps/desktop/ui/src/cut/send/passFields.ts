// SPDX-License-Identifier: GPL-3.0-or-later
import type { SettingsRanges } from "../../ipc";
import { effectiveSettings, type Caps, type PassVm, type PresetLookup } from "../viewmodel";

export type PassIssue = { index: number; field: "speed" | "force" | "repeat"; message: string };

function outOfRange(value: number | null, range: { min: number; max: number }): string | null {
  // Whole numbers only: the cut command deserializes speed, force and repeat as `u32`,
  // so 1.5 fails there instead of being highlighted on the row.
  if (value === null || !Number.isFinite(value) || !Number.isInteger(value) || value < range.min || value > range.max) {
    return `Use ${range.min}–${range.max}`;
  }
  return null;
}

/**
 * Blank or out-of-range speed, force and repeat on an enabled pass. The bounds are the
 * `settings_ranges` answer; with none loaded there is nothing honest to refuse against.
 *
 * A preset the list has not answered for is not blank: the file still holds the numbers, and
 * the cut path resolves them. A preset the loaded list does not contain is the backend's
 * `unknown_preset` refusal, not a form error — blocking a null there would hide that refusal.
 * A number the operator already typed is theirs, and it is checked either way.
 */
export function collectPassIssues(
  rows: PassVm[],
  lookup: PresetLookup,
  caps: Caps,
  ranges: SettingsRanges | null,
): PassIssue[] {
  if (ranges === null) return [];
  const issues: PassIssue[] = [];
  rows.forEach((row, index) => {
    if (!row.enabled) return;
    const named = row.presetId !== null;
    const held = named && lookup.presets.some((preset) => preset.id === row.presetId);
    // Nulls stay the file's problem. An override is already a number this form would send.
    if (named && !held) {
      if (caps.supportsSpeed && row.speed !== null) {
        const message = outOfRange(row.speed, ranges.speed);
        if (message) issues.push({ index, field: "speed", message });
      }
      if (caps.supportsForce && row.force !== null) {
        const message = outOfRange(row.force, ranges.force);
        if (message) issues.push({ index, field: "force", message });
      }
      if (row.repeatCount !== null) {
        const message = outOfRange(row.repeatCount, ranges.repeatCount);
        if (message) issues.push({ index, field: "repeat", message });
      }
      return;
    }
    const effective = effectiveSettings(row, lookup);
    if (caps.supportsSpeed) {
      const message = outOfRange(effective.speed, ranges.speed);
      if (message) issues.push({ index, field: "speed", message });
    }
    if (caps.supportsForce) {
      const message = outOfRange(effective.force, ranges.force);
      if (message) issues.push({ index, field: "force", message });
    }
    const repeat = outOfRange(effective.repeatCount, ranges.repeatCount);
    if (repeat) issues.push({ index, field: "repeat", message: repeat });
  });
  return issues;
}
