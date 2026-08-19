// SPDX-License-Identifier: GPL-3.0-or-later
import type { SettingsRanges } from "../ipc";
import type { Caps, Preset } from "./viewmodel";

/**
 * What the preset editor holds while it is being typed into. A draft is not a `Preset`: its
 * numbers can be blank mid-edit, and its `id` is empty until the entry has been written.
 *
 * The id is minted once, at the first save, and never changes afterwards — a rename rewrites the
 * name alone. A Node's `PresetAssignment` and a `PassKey` (`preset:<id>`) both name a preset by
 * its id, so an id that moved with the name would silently orphan every document that had
 * assigned it.
 */
export type PresetDraft = {
  id: string;
  name: string;
  speed: number | null;
  force: number | null;
  repeatCount: number | null;
};

/** Which of the editor's controls apply, derived rather than tracked: a mode held beside the draft
 *  is a second source of truth for the one question "what is being edited". */
export type EditorMode = "empty" | "builtin" | "user" | "creating";

export function draftOf(p: Preset): PresetDraft {
  return {
    id: p.id,
    name: p.name,
    speed: p.settings.speed,
    force: p.settings.force,
    repeatCount: p.settings.repeat_count,
  };
}

/** A blank entry, repeated once — the only setting a preset cannot leave to the cutter. */
export function newDraft(ranges: SettingsRanges): PresetDraft {
  return { id: "", name: "", speed: null, force: null, repeatCount: ranges.repeatCount.min };
}

export function editorMode(draft: PresetDraft | null, presets: Preset[]): EditorMode {
  if (draft === null) return "empty";
  if (draft.id === "") return "creating";
  return presets.some((p) => p.id === draft.id && p.builtin) ? "builtin" : "user";
}

export function isDirty(draft: PresetDraft, baseline: PresetDraft): boolean {
  return (
    draft.name !== baseline.name ||
    draft.speed !== baseline.speed ||
    draft.force !== baseline.force ||
    draft.repeatCount !== baseline.repeatCount
  );
}

/** Names are compared trimmed and case-folded: the picker is the only place a preset's name is
 *  read, and two entries differing by a space or a capital invite an operator to overwrite the
 *  wrong material. Folded with `toLowerCase`, not the locale's rules: a Turkish `I` folds to `ı`
 *  there, so the same two names would clash on one operator's machine and not on another's. */
const sameName = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/**
 * What refuses this draft, or `null`. First fault wins, so the editor names one thing to fix.
 *
 * The ranges come from Rust (`settings_ranges`), never from constants here: `cutplan::preflight`
 * is what refuses a cut over them, and a second copy on this side would drift into an editor that
 * saves what the cut path then rejects.
 *
 * Range-checked whether or not this cutter honours the field, because the backend checks a stored
 * preset whole — a preset outlives the machine it was typed on, and the file is portable.
 */
export function draftFault(
  draft: PresetDraft,
  presets: Preset[],
  ranges: SettingsRanges,
): string | null {
  if (draft.name.trim() === "") return "A preset needs a name.";
  const clash = presets.find((p) => p.id !== draft.id && sameName(p.name, draft.name));
  if (clash) {
    return clash.builtin
      ? `This cutter ships a built-in preset called "${clash.name}".`
      : `Another preset for this cutter is already called "${clash.name}".`;
  }
  if (draft.repeatCount === null) return "A preset needs a repeat count.";
  const bad = (v: number | null, r: { min: number; max: number }) =>
    v !== null && (!Number.isInteger(v) || v < r.min || v > r.max);
  if (bad(draft.repeatCount, ranges.repeatCount))
    return `Repeat count must be a whole number from ${ranges.repeatCount.min} to ${ranges.repeatCount.max}.`;
  if (bad(draft.speed, ranges.speed))
    return `Speed must be a whole number from ${ranges.speed.min} to ${ranges.speed.max}.`;
  if (bad(draft.force, ranges.force))
    return `Force must be a whole number from ${ranges.force.min} to ${ranges.force.max}.`;
  return null;
}

/**
 * The id a new entry is written under: the name, slugged, made unique against *every* preset this
 * cutter has — the builtins included. An id equal to a builtin's shadows it in `load_presets`,
 * which is a shipped material the operator cannot get back, and the backend refuses that pair
 * outright.
 *
 * A name with nothing sluggable in it (a name in a non-Latin script, say) still needs an id, and
 * `preset` is a word, not a reserved value: it is uniquified like any other.
 */
export function freshPresetId(name: string, presets: Preset[]): string {
  // Latin letters and digits only: an id is a `PassKey`'s tail (`preset:<id>`) and travels through
  // the CLI, so it stays in the character set every one of those spellings can carry. Lowered
  // without the locale's rules for the same reason `sameName` is: a Turkish `I` lowers to `ı`,
  // which this then strips, so one operator's `HTV` would be `htv` and another's `preset`.
  const base =
    name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "preset";
  const taken = new Set(presets.map((p) => p.id));
  if (!taken.has(base)) return base;
  for (let n = 2; ; n++) {
    const candidate = `${base}-${n}`;
    if (!taken.has(candidate)) return candidate;
  }
}

/** `HTV (copy)`, then `HTV (copy 2)` — a copy that arrived under the source's own name would be
 *  refused by `draftFault` the moment it was saved. */
export function copyName(name: string, presets: Preset[]): string {
  const first = `${name.trim()} (copy)`;
  const taken = (candidate: string) => presets.some((p) => sameName(p.name, candidate));
  if (!taken(first)) return first;
  for (let n = 2; ; n++) {
    const candidate = `${name.trim()} (copy ${n})`;
    if (!taken(candidate)) return candidate;
  }
}

/** A copy of `source` as an unwritten draft. Its id is empty because a copy is a new entry: taking
 *  the source's id would rewrite the source, and for a builtin it would shadow it. */
export function copyDraft(source: Preset, presets: Preset[]): PresetDraft {
  return { ...draftOf(source), id: "", name: copyName(source.name, presets) };
}

/** The draft as the entry to write. `builtin: false` is what the on-disk contract says a user
 *  entry is; the backend forces it too, and neither side trusts the other for it. */
export function toPreset(draft: PresetDraft, machineId: string, presets: Preset[]): Preset {
  return {
    id: draft.id === "" ? freshPresetId(draft.name, presets) : draft.id,
    name: draft.name.trim(),
    machine_id: machineId,
    settings: {
      speed: draft.speed,
      force: draft.force,
      // Guarded by `draftFault`, which refuses a blank repeat count before a save can reach here.
      repeat_count: draft.repeatCount ?? 1,
    },
    builtin: false,
  };
}

/** What to select once `deletedId` is gone, given the list as it still stands: the entry after it,
 *  else the one before it, else nothing. Something must be selected, or the editor answers a
 *  delete by showing the settings of a preset that no longer exists. */
export function selectAfterDelete(presets: Preset[], deletedId: string): string | null {
  const i = presets.findIndex((p) => p.id === deletedId);
  if (i < 0) return presets[0]?.id ?? null;
  return presets[i + 1]?.id ?? presets[i - 1]?.id ?? null;
}

/**
 * What a pass cut with this preset and nothing typed over it would use — the mirror of
 * `cutplan::presets::resolve_settings` against an empty `SettingsOverride`.
 *
 * A field the cutter takes from its own panel reads as that rather than as a number, whether the
 * preset leaves it unset or the machine ignores it: both reach the wire the same way, and a bare
 * "20" beside a Puma would promise a force this cut cannot set.
 */
export function presetPreview(draft: PresetDraft, caps: Caps): string {
  const setting = (v: number | null, supported: boolean) =>
    v !== null && supported ? String(v) : "from the cutter's panel";
  const passes =
    draft.repeatCount === null
      ? "no repeat count"
      : draft.repeatCount === 1
        ? "one pass"
        : `${draft.repeatCount} passes`;
  return `Cuts at speed ${setting(draft.speed, caps.supportsSpeed)}, force ${setting(draft.force, caps.supportsForce)}, ${passes}.`;
}
