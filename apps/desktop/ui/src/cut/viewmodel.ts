// SPDX-License-Identifier: GPL-3.0-or-later
// One Bounds for the whole UI: the renderer's. This module re-exports it so cut-preview
// callers keep their import path without a second structurally-identical type drifting.
import type { Bounds } from "../render/hittest";
import type { Grouping, PassKey } from "../ipc";
export type { Bounds };

// View model types (UI representation)
export type PassVm = {
  key: PassKey;
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
  key: PassKey;
  enabled: boolean;
  preset_id: string | null;
  speed: number | null;
  force: number | null;
  repeat_count: number | null;
};

export type CutRequest = {
  device_instance_id: string;
  doc_revision: string;
  grouping: Grouping;
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

/** World-mm → canvas-px mapping for the cut preview: screen = world * scale + t. */
export type Viewport = { scale: number; tx: number; ty: number };

/**
 * Union of the geometry the preview will actually draw: pass-member shape bounds
 * plus travel endpoints. Travel counts because the park/origin moves routinely
 * leave the content box, and a fit that crops them defeats the dashed lines'
 * purpose. Returns null when there is nothing to draw (the empty-plan case).
 */
export function contentBounds(
  shapeBounds: Bounds[],
  travel: [number, number, number, number][]
): Bounds | null {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const b of shapeBounds) {
    minX = Math.min(minX, b.x); minY = Math.min(minY, b.y);
    maxX = Math.max(maxX, b.x + b.w); maxY = Math.max(maxY, b.y + b.h);
  }
  for (const [x1, y1, x2, y2] of travel) {
    minX = Math.min(minX, x1, x2); minY = Math.min(minY, y1, y2);
    maxX = Math.max(maxX, x1, x2); maxY = Math.max(maxY, y1, y2);
  }
  if (minX === Infinity) return null;
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
}

/**
 * Fit `content` (falling back to the artboard for an empty plan) into the canvas
 * with a uniform scale and a symmetric pixel margin, centered on both axes.
 * Degenerate extents are widened to 1mm so a single dot fits at a finite scale
 * instead of an Infinity that blanks the canvas.
 */
export function fitViewport(
  content: Bounds | null,
  artboard: Bounds,
  canvas: { w: number; h: number },
  marginPx: number
): Viewport {
  const target = content ?? artboard;
  const minMm = 1;
  const w = Math.max(target.w, minMm);
  const h = Math.max(target.h, minMm);
  // Widening keeps the original box centered, so re-center the target too.
  const cx = target.x + target.w / 2;
  const cy = target.y + target.h / 2;
  // A margin that swallows the canvas would flip the scale negative and invert the
  // render; clamp the drawable area so scale stays positive and finite for any input.
  const availW = Math.max(canvas.w - 2 * marginPx, 1);
  const availH = Math.max(canvas.h - 2 * marginPx, 1);
  const scale = Math.min(availW / w, availH / h);
  return { scale, tx: canvas.w / 2 - cx * scale, ty: canvas.h / 2 - cy * scale };
}

/**
 * Which canvas edges cut the artboard off. The artboard rect itself is drawn
 * clipped for free by the canvas; these flags drive the explicit "it continues
 * past here" treatment so a missing border edge reads as clipped, not absent.
 */
export function clippedEdges(
  vp: Viewport,
  artboard: Bounds,
  canvas: { w: number; h: number }
): { left: boolean; right: boolean; top: boolean; bottom: boolean } {
  // Half-pixel tolerance: an artboard that mathematically lands on the canvas edge
  // can overhang by float residue (~1e-14), and a sub-pixel overhang is not clipped.
  const eps = 0.5;
  const x1 = artboard.x * vp.scale + vp.tx;
  const y1 = artboard.y * vp.scale + vp.ty;
  const x2 = (artboard.x + artboard.w) * vp.scale + vp.tx;
  const y2 = (artboard.y + artboard.h) * vp.scale + vp.ty;
  return { left: x1 < -eps, right: x2 > canvas.w + eps, top: y1 < -eps, bottom: y2 > canvas.h + eps };
}

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
 * The swapped rows, or null when the move clamps at a boundary — no row moved, so there is
 * nothing to redraw and no reason to spend an IPC round trip.
 */
export function reorderForReplan<T>(rows: T[], index: number, dir: -1 | 1): T[] | null {
  const next = reorderPass(rows, index, dir);
  return next === rows ? null : next;
}

/** What a `PassKey` says, for the two things the UI needs from inside one: a swatch needs the
 *  RGBA, a row label and a row's settings need the preset id. The mirror of
 *  `cutplan::PassKey::from_str`; the example table in `viewmodel.test.ts` keeps the two agreed. */
export type ParsedPassKey =
  | { kind: "all" }
  | { kind: "color"; color: number | null }
  | { kind: "preset"; presetId: string | null }
  | { kind: "unknown"; raw: string };

export function parsePassKey(key: PassKey): ParsedPassKey {
  if (key === "all") return { kind: "all" };
  if (key === "no-color") return { kind: "color", color: null };
  if (key === "no-preset") return { kind: "preset", presetId: null };
  // First separator only, so a preset id may contain one — same rule as the Rust parser.
  const at = key.indexOf(":");
  if (at === -1) return { kind: "unknown", raw: key };
  const mode = key.slice(0, at);
  const value = key.slice(at + 1);
  if (mode === "color") {
    // Eight digits exactly: anything shorter would parse to a colour no shape carries.
    if (/^[0-9a-fA-F]{8}$/.test(value)) return { kind: "color", color: parseInt(value, 16) };
    return { kind: "unknown", raw: key };
  }
  // An empty id parses, because `cutplan::PassKey`'s parser accepts it and the two grammars have
  // to agree: rejecting it here turned a preset-keyed row into an unkeyed one, `presetIdForKey`
  // returned null, the request carried no preset, and `prepare_cut` skipped its lookup and cut
  // with default speed and force. A grammar that fails open on a knife is worse than one that
  // accepts a silly id and lets the refusal fire.
  if (mode === "preset") return { kind: "preset", presetId: value };
  return { kind: "unknown", raw: key };
}

/** The entries a row resolves its material in, and whether they are the aimed cutter's own answer.
 *  Two of the states are the same empty array — a cutter with no presets, and a cutter whose list
 *  this dialog does not hold — so a row handed only `Preset[]` says a pass has no material when what
 *  it has is a material whose name is not available (#267).
 *
 *  `loaded` is false for every way of not holding the list, and there are three: a read in flight
 *  (the seconds after a connect), a read that failed and stands failed, and no cutter aimed at all —
 *  presets are machine-scoped, so with nothing connected there is nothing that could answer. The
 *  row must not distinguish them: it would have to claim a read is happening in the two cases where
 *  none is (Greptile on PR #272). The section above the rows is what names the reason. */
export type PresetLookup = { presets: Preset[]; loaded: boolean };

/** The list the dialog holds for the cutter it is aimed at, and whether it is still that cutter's
 *  answer. `current` goes false the moment a write lands: the presets file has changed, and the
 *  re-read that will say how is still out, so what is held is every entry as it was *before* the
 *  write. Held rather than dropped, because the editor above the rows mints an id and refuses a
 *  colliding name against exactly this list — dropped, the editor would be withheld on every save. */
export type AimedPresets = { presets: Preset[]; current: boolean };

/** The lookup a pass row may price a pass from, which is only ever a current list. A row that
 *  resolved a material in a superseded one would name the preset and show the speed, force and
 *  repeat it had before the write, while `prepare_cut` resolves the new ones from the file — the
 *  dialog and the blade disagreeing about what is about to happen (#274).
 *
 *  Superseded reads as unread rather than as its own marker, because that is what it is: an answer
 *  is owed and none has arrived. It is also where a failed re-read stays, since only a list that
 *  arrives replaces one — so the rows never go back to pricing from the pre-write entries. */
export function rowPresetLookup(aimed: AimedPresets | null): PresetLookup {
  // Emptied, not passed through with `loaded: false`: `presetLabel` answers with a name it finds
  // whatever the flag says, and the name it would find here is the superseded one.
  return aimed !== null && aimed.current
    ? { presets: aimed.presets, loaded: true }
    : { presets: [], loaded: false };
}

/** What a row calls the preset it names. An id the list has answered for and does not hold is
 *  genuinely unknown — presets are machine-scoped and a user entry can be deleted while a document
 *  still names it. An id resolved against a list this dialog does not hold is not unknown, and
 *  saying so would report a material as gone on the strength of a lookup that never happened. */
function presetLabel(presetId: string, { presets, loaded }: PresetLookup): string {
  const preset = presets.find((p) => p.id === presetId);
  if (preset) return preset.name;
  return loaded ? `${presetId} (unknown preset)` : `${presetId} (name unread)`;
}

/** Everything a row's material picker renders, and which option is selected.
 *
 *  Values are `PassKey` spellings rather than bare ids, for the reason the grammar has a
 *  `no-preset` token at all: an id is an unrestricted operator string, so a bare-id picker has to
 *  spend one of them — the empty string — as its "no preset" sentinel, and a pass keyed on a preset
 *  actually called `""` then selects "No preset" no matter what else is offered (Copilot on
 *  PR #272). `no-preset` and `preset:` cannot collide.
 *
 *  The pass's own preset is always among the options, even when the list does not hold it: a
 *  `select` whose value matches no option renders blank, and blank reads as "no preset" for a pass
 *  that has one. */
export function presetPicker(
  presetId: string | null,
  lookup: PresetLookup,
): { selected: PassKey; options: { value: PassKey; label: string }[] } {
  const key = (id: string | null): PassKey => (id === null ? "no-preset" : `preset:${id}`);
  const options = [{ value: key(null), label: "No preset" }];
  if (presetId !== null && !lookup.presets.some((p) => p.id === presetId)) {
    options.push({ value: key(presetId), label: presetLabel(presetId, lookup) });
  }
  for (const preset of lookup.presets) {
    options.push({ value: key(preset.id), label: preset.name });
  }
  return { selected: key(presetId), options };
}

/** How a pass row identifies itself: a swatch when the key is a colour, words otherwise.
 *  Grouping-aware because `no-color` means something different per mode — under Stroke it can
 *  hold brightly filled shapes, so calling it "no visible paint" would be false. */
export function passRowLabel(
  key: PassKey,
  lookup: PresetLookup,
  grouping: Grouping,
): { swatch: string | null; text: string | null } {
  const parsed = parsePassKey(key);
  switch (parsed.kind) {
    case "color":
      if (parsed.color !== null) {
        // Drop the alpha byte: a swatch is a colour, and 0-alpha keys never reach here.
        return { swatch: `#${(parsed.color >>> 8).toString(16).padStart(6, "0")}`, text: null };
      }
      return {
        swatch: null,
        text: grouping === "Stroke" ? "No visible stroke"
            : grouping === "Fill" ? "No visible fill"
            : "No visible paint",
      };
    // Not "every shape": a NoCut shape is excluded from it and counted as skipped.
    case "all":
      return { swatch: null, text: "Every cut shape" };
    case "preset":
      if (parsed.presetId === null) return { swatch: null, text: "No preset" };
      return { swatch: null, text: presetLabel(parsed.presetId, lookup) };
    case "unknown":
      return { swatch: null, text: parsed.raw };
  }
}

/** The preset a pass is keyed on, which is the preset it must be cut with. `prepare_cut`
 *  resolves settings from `preset_id` alone, so a preset-keyed row that arrives without one is
 *  cut with defaults — the operator groups by material and gets none of that material's
 *  settings. Kept even when it resolves to nothing, so the request still says what the
 *  document said. */
export function presetIdForKey(key: PassKey): string | null {
  const parsed = parsePassKey(key);
  return parsed.kind === "preset" ? parsed.presetId : null;
}

/**
 * The pass list to ask the planner for travel in: every planned pass, in dialog order,
 * carrying whether it is cut. Disabled passes are named rather than dropped — the backend
 * skips them when routing the head but still checks that no pass went missing, which a
 * filtered list would make impossible to tell from a frontend bug.
 */
export function toTravelPasses<T extends { key: PassKey; enabled: boolean }>(
  rows: T[],
): { key: PassKey; enabled: boolean }[] {
  return rows.map((r) => ({ key: r.key, enabled: r.enabled }));
}

/**
 * Compute effective settings for a pass, accounting for overrides and presets.
 * Priority: pass override > preset > default (repeatCount defaults to 1)
 *
 * Every field is deferred — `null`, meaning "whatever resolves" — while the pass names a preset
 * the lookup has not answered for. The default repeat of 1 is a claim about the blade ("this pass
 * runs once") and a material with two passes would be reported as one, which is the row saying
 * something it does not know rather than nothing (#267).
 *
 * @param p The pass
 * @param lookup The presets to resolve in, and whether they are the aimed cutter's own
 * @returns Effective settings with resolved speed, force, and repeatCount
 */
export function effectiveSettings(
  p: PassVm,
  lookup: PresetLookup,
): {
  speed: number | null;
  force: number | null;
  repeatCount: number | null;
} {
  // Explicit null, not truthiness: an empty id is a *named* preset, and treating it as absent
  // showed the operator default speed and force while the cut path resolved the real entry — the
  // dialog and the machine disagreeing about what the blade would do.
  const preset = p.presetId !== null ? lookup.presets.find((pr) => pr.id === p.presetId) : null;
  const unread = p.presetId !== null && preset === undefined && !lookup.loaded;

  // Speed: pass override > preset > null
  const speed =
    p.speed !== null ? p.speed : preset?.settings.speed ?? null;

  // Force: pass override > preset > null
  const force =
    p.force !== null ? p.force : preset?.settings.force ?? null;

  // RepeatCount: pass override > preset > 1, and nothing at all until the preset can be resolved
  const repeatCount =
    p.repeatCount !== null
      ? p.repeatCount
      : unread ? null : preset?.settings.repeat_count ?? 1;

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
 * Convert PassVm[] to CutRequest for transmission to Rust backend.
 * Maps camelCase PassVm to snake_case ConfiguredPassDto fields.
 *
 * The grouping travels with the rows because it is what named them: rows keyed under one mode
 * sent under another would match passes holding different shapes.
 */
export function toCutRequest(
  deviceInstanceId: string,
  docRevision: string,
  grouping: Grouping,
  passes: PassVm[]
): CutRequest {
  return {
    device_instance_id: deviceInstanceId,
    doc_revision: docRevision,
    grouping,
    passes: passes.map((p) => ({
      key: p.key,
      enabled: p.enabled,
      preset_id: p.presetId,
      speed: p.speed,
      force: p.force,
      repeat_count: p.repeatCount,
    })),
  };
}
