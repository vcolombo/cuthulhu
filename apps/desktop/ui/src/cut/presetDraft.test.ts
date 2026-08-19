// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import type { SettingsRanges } from "../ipc";
import {
  copyDraft,
  copyName,
  draftFault,
  draftOf,
  editorMode,
  freshPresetId,
  isDirty,
  newDraft,
  presetPreview,
  selectAfterDelete,
  toPreset,
  type PresetDraft,
} from "./presetDraft";
import type { Caps, Preset } from "./viewmodel";

// The bounds `cutplan::preflight::SETTINGS_RANGES` publishes, as `settings_ranges` hands them over.
// Written here as a fixture, not as a default: nothing in the editor may fall back to them.
const RANGES: SettingsRanges = {
  speed: { min: 1, max: 30 },
  force: { min: 1, max: 33 },
  repeatCount: { min: 1, max: 10 },
};

const CAMEO: Caps = { supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false };
const PUMA: Caps = { supportsSpeed: false, supportsForce: false, needsOperatorPassConfirm: true };

const preset = (over: Partial<Preset> & { id: string }): Preset => ({
  name: over.id,
  machine_id: "cameo5",
  settings: { speed: 5, force: 20, repeat_count: 1 },
  builtin: false,
  ...over,
});

const HTV = preset({ id: "cameo5-htv", name: "HTV", builtin: true });
const MINE = preset({ id: "my-vinyl", name: "My Vinyl" });

describe("editorMode", () => {
  it("tells a builtin from an entry of the operator's own, and an unwritten one from both", () => {
    const presets = [HTV, MINE];
    expect(editorMode(null, presets)).toBe("empty");
    expect(editorMode(draftOf(HTV), presets)).toBe("builtin");
    expect(editorMode(draftOf(MINE), presets)).toBe("user");
    expect(editorMode(newDraft(RANGES), presets)).toBe("creating");
  });
});

describe("isDirty", () => {
  it("is the difference between the draft and the stored entry, field by field", () => {
    const baseline = draftOf(MINE);
    expect(isDirty(baseline, baseline)).toBe(false);
    expect(isDirty({ ...baseline, name: "Other" }, baseline)).toBe(true);
    expect(isDirty({ ...baseline, speed: 6 }, baseline)).toBe(true);
    expect(isDirty({ ...baseline, force: null }, baseline)).toBe(true);
    expect(isDirty({ ...baseline, repeatCount: 2 }, baseline)).toBe(true);
  });

  it("is false for a blank new entry nobody has typed into, so closing the dialog asks nothing", () => {
    expect(isDirty(newDraft(RANGES), newDraft(RANGES))).toBe(false);
  });
});

describe("draftFault", () => {
  const draft = (over: Partial<PresetDraft>): PresetDraft => ({ ...draftOf(MINE), ...over });

  it("refuses a nameless preset — the name is what the picker shows", () => {
    expect(draftFault(draft({ name: "   " }), [MINE], RANGES)).toContain("needs a name");
  });

  it("refuses a name another of this cutter's entries already has, naming which", () => {
    const other = preset({ id: "other", name: "Cardstock" });
    const fault = draftFault(draft({ name: "Cardstock" }), [MINE, other], RANGES);
    expect(fault).toContain("Cardstock");
    expect(fault).toContain("already called");
  });

  it("says so when the clashing name is a builtin's, because that one cannot be renamed away", () => {
    expect(draftFault(draft({ name: "HTV" }), [MINE, HTV], RANGES)).toContain("built-in");
  });

  it("compares names trimmed and case-folded, so a space or a capital is not a second Vinyl", () => {
    expect(draftFault(draft({ id: "new", name: " my vinyl " }), [MINE], RANGES)).toContain(
      "already called",
    );
  });

  it("accepts the edited entry's own name — a rename that changes nothing else is not a clash", () => {
    expect(draftFault(draft({ name: "My Vinyl", speed: 9 }), [MINE], RANGES)).toBeNull();
  });

  it("refuses settings outside the ranges the backend was asked for, quoting them", () => {
    expect(draftFault(draft({ speed: 31 }), [MINE], RANGES)).toContain("1 to 30");
    expect(draftFault(draft({ force: 0 }), [MINE], RANGES)).toContain("1 to 33");
    expect(draftFault(draft({ repeatCount: 11 }), [MINE], RANGES)).toContain("1 to 10");
    expect(draftFault(draft({ repeatCount: null }), [MINE], RANGES)).toContain("repeat count");
  });

  it("refuses a fraction: the wire carries whole numbers, so 2.5 passes is not a request", () => {
    expect(draftFault(draft({ speed: 2.5 }), [MINE], RANGES)).toContain("whole number");
  });

  it("accepts speed and force left unset — that is the cutter's own panel, not a missing value", () => {
    expect(draftFault(draft({ speed: null, force: null }), [MINE], RANGES)).toBeNull();
  });
});

describe("freshPresetId", () => {
  it("slugs the name", () => {
    expect(freshPresetId("Thick Card #2", [])).toBe("thick-card-2");
  });

  it("never lands on an id this cutter already has, builtins included", () => {
    // The whole point: an entry saved under a builtin's id shadows it in `load_presets`, and the
    // backend refuses that pair — so the id has to dodge every id already listed, not just the
    // operator's own.
    const builtinHtv = preset({ id: "htv", name: "HTV", builtin: true });
    expect(freshPresetId("HTV", [builtinHtv])).toBe("htv-2");
    expect(freshPresetId("HTV", [builtinHtv, preset({ id: "htv-2" })])).toBe("htv-3");
    expect(freshPresetId("HTV", [HTV])).toBe("htv");
  });

  it("still yields an id for a name with nothing sluggable in it", () => {
    expect(freshPresetId("厚紙", [])).toBe("preset");
    expect(freshPresetId("厚紙", [preset({ id: "preset" })])).toBe("preset-2");
  });
});

describe("copyName and copyDraft", () => {
  it("names a copy so it cannot collide with the entry it came from", () => {
    expect(copyName("HTV", [HTV])).toBe("HTV (copy)");
    expect(copyName("HTV", [HTV, preset({ id: "c1", name: "HTV (copy)" })])).toBe("HTV (copy 2)");
  });

  it("copies a builtin's settings into an unwritten entry, never its id", () => {
    const copy = copyDraft(HTV, [HTV]);
    expect(copy.id).toBe("");
    expect(copy.name).toBe("HTV (copy)");
    expect(copy.speed).toBe(HTV.settings.speed);
    expect(copy.force).toBe(HTV.settings.force);
    // And what it is saved as is a user entry under a fresh id, on the same machine.
    const written = toPreset(copy, "cameo5", [HTV]);
    expect(written.builtin).toBe(false);
    expect(written.id).not.toBe(HTV.id);
    expect(written.machine_id).toBe("cameo5");
  });
});

describe("toPreset", () => {
  it("keeps the id of an entry that already has one, so a rename cannot orphan an assignment", () => {
    // A Node's PresetAssignment and a PassKey (`preset:<id>`) both name a preset by its id.
    const renamed = toPreset({ ...draftOf(MINE), name: "Renamed" }, "cameo5", [MINE]);
    expect(renamed.id).toBe(MINE.id);
    expect(renamed.name).toBe("Renamed");
  });

  it("trims the name and carries the machine it was edited for", () => {
    const p = toPreset({ ...newDraft(RANGES), name: "  Card  " }, "puma", []);
    expect(p.name).toBe("Card");
    expect(p.machine_id).toBe("puma");
    expect(p.settings.repeat_count).toBe(1);
  });
});

describe("selectAfterDelete", () => {
  const a = preset({ id: "a" });
  const b = preset({ id: "b" });
  const c = preset({ id: "c" });

  it("moves to the following entry, falls back to the previous one, and to nothing at all", () => {
    expect(selectAfterDelete([a, b, c], "b")).toBe("c");
    expect(selectAfterDelete([a, b, c], "c")).toBe("b");
    expect(selectAfterDelete([a], "a")).toBeNull();
  });
});

describe("presetPreview", () => {
  it("reads back the settings a pass would cut with when nothing is typed over them", () => {
    expect(presetPreview(draftOf(MINE), CAMEO)).toBe("Cuts at speed 5, force 20, one pass.");
    expect(presetPreview({ ...draftOf(MINE), repeatCount: 3 }, CAMEO)).toContain("3 passes");
  });

  it("says the cutter's panel for a field this machine takes from there, however it was stored", () => {
    // A Puma's speed never reaches the wire, so a bare "5" beside one promises a setting this cut
    // cannot make.
    expect(presetPreview(draftOf(MINE), PUMA)).toBe(
      "Cuts at speed from the cutter's panel, force from the cutter's panel, one pass.",
    );
    expect(presetPreview({ ...draftOf(MINE), speed: null }, CAMEO)).toContain(
      "speed from the cutter's panel",
    );
  });
});
