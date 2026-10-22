// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import type { SettingsRanges } from "../../ipc";
import type { Caps, PassVm, Preset, PresetLookup } from "../viewmodel";
import { collectPassIssues } from "./passFields";

const RANGES: SettingsRanges = {
  speed: { min: 1, max: 30 },
  force: { min: 1, max: 33 },
  repeatCount: { min: 1, max: 10 },
  weed: {
    margin_mm: { min: 0.5, max: 50 },
    spacing_mm: { min: 1, max: 500 },
    clearance_mm: { min: 0, max: 50 },
  },
  weedDefaults: { margin_mm: 3, lines: "None", spacing_mm: 10, clearance_mm: 1 },
};

const CAMEO: Caps = { supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false };
const PUMA: Caps = { supportsSpeed: false, supportsForce: false, needsOperatorPassConfirm: true };

function row(patch: Partial<PassVm> = {}): PassVm {
  return {
    key: "color:ff0000ff",
    shapeCount: 1,
    enabled: true,
    presetId: null,
    speed: null,
    force: null,
    repeatCount: null,
    ...patch,
  };
}

function lookup(presets: Preset[] = [], loaded = true): PresetLookup {
  return { presets, loaded };
}

function preset(speed: number | null, force: number | null, repeat = 1): Preset {
  return {
    id: "vinyl",
    name: "Vinyl",
    machine_id: "cameo5",
    settings: { speed, force, repeat_count: repeat },
    builtin: false,
  };
}

describe("collectPassIssues", () => {
  it("refuses a blank speed and force when the machine can set them", () => {
    const issues = collectPassIssues([row()], lookup(), CAMEO, RANGES);
    expect(issues).toEqual([
      { index: 0, field: "speed", message: "Use 1–30" },
      { index: 0, field: "force", message: "Use 1–33" },
    ]);
  });

  it("does not invent a speed error for a machine that cannot set one", () => {
    expect(collectPassIssues([row()], lookup(), PUMA, RANGES)).toEqual([]);
  });

  it("accepts a preset's own speed and refuses one the preset leaves empty", () => {
    const filled = collectPassIssues([row({ presetId: "vinyl" })], lookup([preset(5, 12)]), CAMEO, RANGES);
    expect(filled).toEqual([]);
    const empty = collectPassIssues([row({ presetId: "vinyl" })], lookup([preset(null, null)]), CAMEO, RANGES);
    expect(empty.map((issue) => issue.field)).toEqual(["speed", "force"]);
  });

  it("does not treat an unread or deleted preset as a blank the form can fill", () => {
    const named = row({ presetId: "vinyl" });
    expect(collectPassIssues([named], lookup([], false), CAMEO, RANGES)).toEqual([]);
    expect(collectPassIssues([named], lookup([], true), CAMEO, RANGES)).toEqual([]);
  });

  it("still refuses a blank no-preset pass beside one whose preset is unread or gone", () => {
    const named = row({ key: "preset:vinyl", presetId: "vinyl" });
    const blank = row({ key: "no-preset", presetId: null });
    const issues = [
      { index: 1, field: "speed", message: "Use 1–30" },
      { index: 1, field: "force", message: "Use 1–33" },
    ];
    expect(collectPassIssues([named, blank], lookup([], false), CAMEO, RANGES)).toEqual(issues);
    expect(collectPassIssues([named, blank], lookup([], true), CAMEO, RANGES)).toEqual(issues);
  });

  it("refuses a speed outside the ranges, including zero", () => {
    expect(collectPassIssues([row({ speed: 99, force: 10 })], lookup(), CAMEO, RANGES)).toEqual([
      { index: 0, field: "speed", message: "Use 1–30" },
    ]);
    expect(collectPassIssues([row({ speed: 0, force: 10 })], lookup(), CAMEO, RANGES)).toEqual([
      { index: 0, field: "speed", message: "Use 1–30" },
    ]);
  });

  it("refuses a repeat past the backend maximum and accepts the default of one", () => {
    expect(collectPassIssues([row({ speed: 10, force: 10, repeatCount: 11 })], lookup(), CAMEO, RANGES)).toEqual([
      { index: 0, field: "repeat", message: "Use 1–10" },
    ]);
    expect(collectPassIssues([row({ speed: 10, force: 10 })], lookup(), CAMEO, RANGES)).toEqual([]);
  });

  it("skips a pass that will not be cut, and skips everything until ranges arrive", () => {
    expect(collectPassIssues([row({ enabled: false })], lookup(), CAMEO, RANGES)).toEqual([]);
    expect(collectPassIssues([row()], lookup(), CAMEO, null)).toEqual([]);
  });
});
