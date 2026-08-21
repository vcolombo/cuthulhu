// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { clippedEdges, contentBounds, fitViewport, type Bounds, type Viewport } from "./viewmodel";

const CANVAS = { w: 400, h: 300 };
const MARGIN = 16;
const ARTBOARD: Bounds = { x: 0, y: 0, w: 330, h: 3000 };

const toScreen = (vp: Viewport, x: number, y: number) => ({
  x: x * vp.scale + vp.tx,
  y: y * vp.scale + vp.ty,
});

describe("contentBounds", () => {
  it("is null for an empty plan", () => {
    expect(contentBounds([], [])).toBeNull();
  });

  it("unions shape bounds with travel endpoints", () => {
    // Both travel endpoints sit outside the shape box, so an implementation that
    // ignores either endpoint produces a smaller union and fails.
    const b = contentBounds([{ x: 10, y: 10, w: 5, h: 5 }], [[0, 0, 30, 40]]);
    expect(b).toEqual({ x: 0, y: 0, w: 30, h: 40 });
  });
});

describe("fitViewport", () => {
  it("fits the artboard when the plan is empty", () => {
    const vp = fitViewport(null, ARTBOARD, CANVAS, MARGIN);
    const top = toScreen(vp, ARTBOARD.x, ARTBOARD.y);
    const bottom = toScreen(vp, ARTBOARD.x + ARTBOARD.w, ARTBOARD.y + ARTBOARD.h);
    expect(vp.scale).toBeGreaterThan(0);
    expect(top.y).toBeCloseTo(MARGIN);
    expect(bottom.y).toBeCloseTo(CANVAS.h - MARGIN);
  });

  it("makes tiny content occupy most of the canvas", () => {
    // A 10×10mm sticker on the Cameo artboard — the case the fixed 1px=1mm
    // mapping rendered as a 10px speck in the corner.
    const vp = fitViewport({ x: 5, y: 5, w: 10, h: 10 }, ARTBOARD, CANVAS, MARGIN);
    const tl = toScreen(vp, 5, 5);
    const br = toScreen(vp, 15, 15);
    expect(br.x - tl.x).toBeCloseTo(CANVAS.h - 2 * MARGIN); // height-limited uniform scale
    expect(tl.x).toBeGreaterThanOrEqual(0);
    expect(br.y).toBeLessThanOrEqual(CANVAS.h);
  });

  it("shrinks content larger than the canvas until fully visible", () => {
    const vp = fitViewport({ x: 0, y: 0, w: 2000, h: 500 }, ARTBOARD, CANVAS, MARGIN);
    expect(vp.scale).toBeLessThan(1);
    const tl = toScreen(vp, 0, 0);
    const br = toScreen(vp, 2000, 500);
    expect(tl.x).toBeCloseTo(MARGIN);
    expect(br.x).toBeCloseTo(CANVAS.w - MARGIN);
    expect(tl.y).toBeGreaterThanOrEqual(0);
    expect(br.y).toBeLessThanOrEqual(CANVAS.h);
  });

  it("centers content offset far from the origin", () => {
    const vp = fitViewport({ x: 300, y: 2900, w: 20, h: 20 }, ARTBOARD, CANVAS, MARGIN);
    const center = toScreen(vp, 310, 2910);
    expect(center.x).toBeCloseTo(CANVAS.w / 2);
    expect(center.y).toBeCloseTo(CANVAS.h / 2);
  });

  it("keeps scale positive and finite when the margin swallows the canvas", () => {
    const vp = fitViewport({ x: 0, y: 0, w: 10, h: 10 }, ARTBOARD, { w: 20, h: 20 }, 50);
    expect(Number.isFinite(vp.scale)).toBe(true);
    expect(vp.scale).toBeGreaterThan(0);
  });

  it("widens degenerate content instead of fitting at Infinity", () => {
    const vp = fitViewport({ x: 50, y: 50, w: 0, h: 0 }, ARTBOARD, CANVAS, MARGIN);
    expect(Number.isFinite(vp.scale)).toBe(true);
    const dot = toScreen(vp, 50, 50);
    expect(dot.x).toBeCloseTo(CANVAS.w / 2);
    expect(dot.y).toBeCloseTo(CANVAS.h / 2);
  });
});

describe("clippedEdges", () => {
  it("reports no clipping when the whole artboard fits", () => {
    const vp = fitViewport(null, ARTBOARD, CANVAS, MARGIN);
    expect(clippedEdges(vp, ARTBOARD, CANVAS)).toEqual({
      left: false, right: false, top: false, bottom: false,
    });
  });

  it("reports the edges a content-fitted view cuts off", () => {
    // Content hugging the artboard's top-left corner (within the fit margin):
    // zooming in keeps the sheet's top-left visible while its right and bottom
    // edges run past the canvas.
    const vp = fitViewport({ x: 0.2, y: 0.2, w: 10, h: 10 }, ARTBOARD, CANVAS, MARGIN);
    expect(clippedEdges(vp, ARTBOARD, CANVAS)).toEqual({
      left: false, right: true, top: false, bottom: true,
    });
  });

  it("reports all edges clipped when content sits mid-sheet", () => {
    const vp = fitViewport({ x: 150, y: 1500, w: 10, h: 10 }, ARTBOARD, CANVAS, MARGIN);
    expect(clippedEdges(vp, ARTBOARD, CANVAS)).toEqual({
      left: true, right: true, top: true, bottom: true,
    });
  });
});
import {
  reorderPass,
  reorderForReplan,
  toTravelPasses,
  effectiveSettings,
  fieldDisabled,
  toCutRequest,
  parsePassKey,
  passRowLabel,
  presetIdForKey,
  unlistedPresetOption,
  type PassVm,
  type Caps,
  type Preset,
} from "./viewmodel";
import type { DeviceInfo, Grouping } from "../ipc";

const aDevice = (): DeviceInfo => ({
  instance_id: "usb:mock",
  machine_id: "cameo5",
  transport: { Usb: { locator: "mock" } },
  candidate: false,
  host: null,
});

describe("reorderPass", () => {
  it("swaps adjacent passes when within bounds", () => {
    const passes: PassVm[] = [
      {
        key: "color:ff0000ff",
        shapeCount: 5,
        enabled: true,
        presetId: "p1",
        speed: 100,
        force: 50,
        repeatCount: 1,
      },
      {
        key: "color:00ff00ff",
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
        key: "color:ff0000ff",
        shapeCount: 5,
        enabled: true,
        presetId: "p1",
        speed: 100,
        force: 50,
        repeatCount: 1,
      },
      {
        key: "color:00ff00ff",
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
        key: "color:ff0000ff",
        shapeCount: 5,
        enabled: true,
        presetId: "p1",
        speed: 100,
        force: 50,
        repeatCount: 1,
      },
      {
        key: "color:00ff00ff",
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

describe("reorderForReplan", () => {
  const rows = [
    { key: "color:ff0000ff", label: "red", enabled: true },
    { key: "color:00ff00ff", label: "green", enabled: true },
    { key: "no-color", label: "uncolored", enabled: true },
  ];

  it("returns the swapped rows", () => {
    const moved = reorderForReplan(rows, 0, 1);
    expect(moved).not.toBeNull();
    expect(moved!.map((r) => r.label)).toEqual(["green", "red", "uncolored"]);
  });

  it("is null at a boundary, so a clamped move costs no replan", () => {
    expect(reorderForReplan(rows, 0, -1)).toBeNull();
    expect(reorderForReplan(rows, 2, 1)).toBeNull();
  });

  it("sends the swapped order to the planner, not the original", () => {
    // Read from the swapped rows: taking the order off the originals replans travel
    // for a list the dialog no longer shows.
    expect(toTravelPasses(reorderForReplan(rows, 0, 1)!).map((p) => p.key)).toEqual([
      "color:00ff00ff", "color:ff0000ff", "no-color",
    ]);
  });
});

describe("toTravelPasses", () => {
  it("names every pass, disabled ones included, carrying whether each is cut", () => {
    // Dropping the disabled pass here would leave the backend unable to tell a pass the
    // operator switched off from one a frontend bug lost.
    const rows = [
      { key: "color:ff0000ff", enabled: false },
      { key: "color:00ff00ff", enabled: true },
    ];
    expect(toTravelPasses(rows)).toEqual([
      { key: "color:ff0000ff", enabled: false },
      { key: "color:00ff00ff", enabled: true },
    ]);
  });
});

/** A list that has been read, which is what every case below but the unread-window ones is about.
 *  Spelled out rather than defaulted in the functions: the two states are the same array, and the
 *  whole point of #267 is that a caller has to say which one it holds. */
const read = (presets: Preset[] = []) => ({ presets, loaded: true });
/** No read has answered for the aimed cutter yet — the seconds after a connect, and for as long as
 *  a failed `list_presets` stands. */
const unread = { presets: [], loaded: false };

describe("effectiveSettings", () => {
  it("uses pass override over preset", () => {
    const pass: PassVm = {
      key: "color:ff0000ff",
      shapeCount: 5,
      enabled: true,
      presetId: "preset1",
      speed: 100,
      force: 50,
      repeatCount: 2,
    };

    const result = effectiveSettings(pass, read());
    expect(result.speed).toBe(100);
    expect(result.force).toBe(50);
    expect(result.repeatCount).toBe(2);
  });

  it("falls back to preset when pass fields are null", () => {
    const pass: PassVm = {
      key: "color:ff0000ff",
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

    const result = effectiveSettings(pass, read(presets));
    expect(result.speed).toBe(150);
    expect(result.force).toBe(75);
    expect(result.repeatCount).toBe(3);
  });

  it("uses default repeatCount=1 when no preset match and pass is null", () => {
    const pass: PassVm = {
      key: "color:ff0000ff",
      shapeCount: 5,
      enabled: true,
      presetId: null,
      speed: null,
      force: null,
      repeatCount: null,
    };

    const result = effectiveSettings(pass, read());
    expect(result.speed).toBeNull();
    expect(result.force).toBeNull();
    expect(result.repeatCount).toBe(1);
  });

  it("handles partial overrides (speed override, force from preset)", () => {
    const pass: PassVm = {
      key: "color:ff0000ff",
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

    const result = effectiveSettings(pass, read(presets));
    expect(result.speed).toBe(100);
    expect(result.force).toBe(75);
    expect(result.repeatCount).toBe(3);
  });

  // The window #267 is about: the row names a material, and the list that would resolve it has not
  // arrived. A repeat of 1 is a claim about the blade — this pass runs once — and a material with
  // two passes reported as one is the dialog and the machine disagreeing about what will happen.
  it("defers every setting while the list that would resolve the preset is unread", () => {
    const pass: PassVm = {
      key: "preset:card-stock",
      shapeCount: 5,
      enabled: true,
      presetId: "card-stock",
      speed: null,
      force: null,
      repeatCount: null,
    };

    expect(effectiveSettings(pass, unread)).toEqual({ speed: null, force: null, repeatCount: null });
  });

  // An override is the operator's own number, not something the list could answer for.
  it("keeps a row's overrides while the list is unread", () => {
    const pass: PassVm = {
      key: "preset:card-stock",
      shapeCount: 5,
      enabled: true,
      presetId: "card-stock",
      speed: 90,
      force: 30,
      repeatCount: 2,
    };

    expect(effectiveSettings(pass, unread)).toEqual({ speed: 90, force: 30, repeatCount: 2 });
  });

  // A pass that names no material has nothing to wait for: `no-preset` is an answer, so the
  // default repeat is a fact about that pass rather than a guess about an unread one.
  it("defaults the repeat of a pass with no preset even while the list is unread", () => {
    const pass: PassVm = {
      key: "no-preset",
      shapeCount: 5,
      enabled: true,
      presetId: null,
      speed: null,
      force: null,
      repeatCount: null,
    };

    expect(effectiveSettings(pass, unread)).toEqual({ speed: null, force: null, repeatCount: 1 });
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

describe("toCutRequest", () => {
  it("serializes PassVm to CutRequest with ConfiguredPassDto fields", () => {
    const passes: PassVm[] = [
      {
        key: "color:ff0000ff",
        shapeCount: 5,
        enabled: true,
        presetId: "preset1",
        speed: 100,
        force: 50,
        repeatCount: 2,
      },
    ];

    const result = toCutRequest("device123", "42", "Color", passes);

    expect(result.device_instance_id).toBe("device123");
    expect(result.doc_revision).toBe("42");
    expect(result.passes).toHaveLength(1);
    expect(result.grouping).toBe("Color");
    expect(result.passes[0]).toEqual({
      key: "color:ff0000ff",
      enabled: true,
      preset_id: "preset1",
      speed: 100,
      force: 50,
      repeat_count: 2,
    });
  });

  it("serializes null values explicitly in ConfiguredPassDto", () => {
    const passes: PassVm[] = [
      {
        key: "no-color",
        shapeCount: 3,
        enabled: false,
        presetId: null,
        speed: null,
        force: null,
        repeatCount: null,
      },
    ];

    const result = toCutRequest("device123", "42", "Color", passes);

    expect(result.passes[0]).toEqual({
      key: "no-color",
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
        key: "color:ff0000ff",
        shapeCount: 5,
        enabled: true,
        presetId: "preset1",
        speed: 100,
        force: 50,
        repeatCount: 1,
      },
      {
        key: "color:00ff00ff",
        shapeCount: 3,
        enabled: true,
        presetId: "preset2",
        speed: 120,
        force: 60,
        repeatCount: 2,
      },
    ];

    const result = toCutRequest("device123", "42", "Color", passes);

    expect(result.passes).toHaveLength(2);
    expect(result.passes[0].preset_id).toBe("preset1");
    expect(result.passes[1].preset_id).toBe("preset2");
  });
});

describe("parsePassKey", () => {
  // The same table as crates/cutplan/src/pass_key.rs's round-trip test. These two tables are
  // the only thing keeping the dialog and the planner agreed on what a pass is called.
  it.each([
    ["all", { kind: "all" }],
    ["color:ff0000ff", { kind: "color", color: 0xff0000ff }],
    ["no-color", { kind: "color", color: null }],
    ["preset:cameo5-htv", { kind: "preset", presetId: "cameo5-htv" }],
    ["no-preset", { kind: "preset", presetId: null }],
  ])("parses %s", (key, expected) => {
    expect(parsePassKey(key as string)).toEqual(expected);
  });

  // Codex's blocking finding on the gate re-run: Rust's parser accepts `preset:`, and a mirror
  // that refused it turned a preset-keyed row into an unkeyed one — the request then carried no
  // preset and the cut used default speed and force instead of being refused. The two grammars
  // agree, so the refusal fires.
  it("parses an empty preset id, as the Rust grammar does", () => {
    expect(parsePassKey("preset:")).toEqual({ kind: "preset", presetId: "" });
    expect(presetIdForKey("preset:")).toBe("");
  });

  it("keeps a colon inside a preset id", () => {
    expect(parsePassKey("preset:vinyl:thin")).toEqual({ kind: "preset", presetId: "vinyl:thin" });
  });

  // The collision the grammar exists to avoid: a preset actually called "none" is not the
  // absence of a preset.
  it("tells a preset called none from no preset at all", () => {
    expect(parsePassKey("preset:none")).toEqual({ kind: "preset", presetId: "none" });
    expect(parsePassKey("no-preset")).toEqual({ kind: "preset", presetId: null });
  });

  // A key the backend produced that this cannot read is a version mismatch, not operator
  // input: it renders as itself rather than throwing, because a dialog that crashes mid-cut is
  // worse than one showing a string nobody recognises.
  it("returns the raw key it cannot parse", () => {
    expect(parsePassKey("line-type:cut")).toEqual({ kind: "unknown", raw: "line-type:cut" });
    expect(parsePassKey("color:ff0000")).toEqual({ kind: "unknown", raw: "color:ff0000" });
  });
});

describe("passRowLabel", () => {
  const presets = [{ id: "cameo5-htv", name: "HTV", machine_id: "cameo5",
                     settings: { speed: 5, force: 20, repeat_count: 1 }, builtin: true }];

  it("names a colour pass by its swatch, not by words", () => {
    expect(passRowLabel("color:ff0000ff", read(presets), "Color")).toEqual({ swatch: "#ff0000", text: null });
  });

  // Grouping-aware, because `no-color` means something different in each colour mode: under
  // Stroke it can hold brightly filled shapes, so "no visible paint" would be false.
  it.each([
    ["Color", "No visible paint"],
    ["Stroke", "No visible stroke"],
    ["Fill", "No visible fill"],
  ])("says what the colourless pass holds under %s", (grouping, text) => {
    expect(passRowLabel("no-color", read(presets), grouping as Grouping)).toEqual({ swatch: null, text });
  });

  // Not "every shape": a NoCut shape is excluded and counted as skipped.
  it("names the single pass for what it holds", () => {
    expect(passRowLabel("all", read(presets), "Single")).toEqual({ swatch: null, text: "Every cut shape" });
  });

  it("resolves a preset to its name", () => {
    expect(passRowLabel("preset:cameo5-htv", read(presets), "Preset")).toEqual({ swatch: null, text: "HTV" });
  });

  // A preset a document names but the file no longer has: the planner keys the pass anyway, so
  // the dialog has to render one.
  it("shows an unresolved preset id as unknown", () => {
    expect(passRowLabel("preset:deleted", read(presets), "Preset"))
      .toEqual({ swatch: null, text: "deleted (unknown preset)" });
  });

  // #267: the same empty-handed lookup, for the opposite reason. "unknown preset" is a claim about
  // the presets file, and a list nobody has read cannot support it — the row would tell an operator
  // their material is gone while the read that names it is still in flight.
  it("says a preset is being read rather than unknown while no list has answered", () => {
    expect(passRowLabel("preset:cameo5-htv", unread, "Preset"))
      .toEqual({ swatch: null, text: "cameo5-htv (reading…)" });
  });

  // A found entry wins over the unread marker, which exists only for a lookup that came back
  // empty-handed: the name is the better answer wherever it came from.
  it("names a material the lookup holds whether or not the read has landed", () => {
    expect(passRowLabel("preset:cameo5-htv", { presets, loaded: false }, "Preset"))
      .toEqual({ swatch: null, text: "HTV" });
  });

  it("names the pass that resolves to no material", () => {
    expect(passRowLabel("no-preset", read(presets), "Preset")).toEqual({ swatch: null, text: "No preset" });
  });
});

// The picker beside the label, which had the same defect in a worse shape: a `select` whose value
// matches no option renders blank, and blank is what "No preset" looks like.
describe("unlistedPresetOption", () => {
  const presets = [{ id: "cameo5-htv", name: "HTV", machine_id: "cameo5",
                     settings: { speed: 5, force: 20, repeat_count: 1 }, builtin: true }];

  it("adds nothing for a preset the list holds", () => {
    expect(unlistedPresetOption("cameo5-htv", read(presets))).toBeNull();
  });

  it("adds nothing for a pass that names no preset", () => {
    expect(unlistedPresetOption(null, read(presets))).toBeNull();
  });

  it("names a deleted preset the read list does not hold", () => {
    expect(unlistedPresetOption("card-stock", read(presets)))
      .toEqual({ value: "card-stock", label: "card-stock (unknown preset)" });
  });

  it("names a preset no list has answered for as being read", () => {
    expect(unlistedPresetOption("card-stock", unread))
      .toEqual({ value: "card-stock", label: "card-stock (reading…)" });
  });

  // The id that reads like no id at all: an empty string is a named preset, and an option carrying
  // it is what keeps the picker off "No preset" for a pass that has one.
  it("names an empty preset id the list does not hold", () => {
    expect(unlistedPresetOption("", unread)).toEqual({ value: "", label: " (reading…)" });
  });
});

describe("presetIdForKey", () => {
  // What makes grouping by material do the thing it exists for: the pass's own preset supplies
  // its settings, instead of the operator re-picking it once per pass.
  it("takes the preset a preset-keyed pass names", () => {
    expect(presetIdForKey("preset:cameo5-htv")).toBe("cameo5-htv");
  });

  // Kept even when it resolves to nothing: prepare_cut falls back to the override-or-default
  // path, and clearing it here would silently drop what the document said.
  it("keeps an id that may not resolve", () => {
    expect(presetIdForKey("preset:deleted")).toBe("deleted");
  });

  it.each(["all", "no-color", "no-preset", "color:ff0000ff"])("has nothing to take from %s", (key) => {
    expect(presetIdForKey(key)).toBeNull();
  });
});

describe("installed plan", () => {
  // The rows and the mode that produced them travel together. A row list is only ever sent
  // with the grouping of the plan it came from, which is what this shape enforces: there is no
  // way to build a request from rows without naming their plan's grouping.
  it("builds a request only from a plan's own grouping and rows", () => {
    const plan = {
      grouping: "Fill" as Grouping,
      revision: "7",
      skippedNotCut: 0,
      rows: [
        { key: "color:00ff00ff", shapeCount: 1, enabled: true, presetId: null,
          speed: null, force: null, repeatCount: null },
      ],
    };
    const request = toCutRequest("dev-1", plan.revision, plan.grouping, plan.rows);
    expect(request.grouping).toBe("Fill");
    expect(request.passes.map((p) => p.key)).toEqual(["color:00ff00ff"]);
  });

  // A preset-grouped plan's rows carry their own preset, which is what makes the pass cut with
  // that material rather than with defaults.
  it("carries each preset-keyed row's own preset into the request", () => {
    const rows = ["preset:cameo5-htv", "no-preset"].map((key) => ({
      key, shapeCount: 1, enabled: true, presetId: presetIdForKey(key),
      speed: null, force: null, repeatCount: null,
    }));
    const request = toCutRequest("dev-1", "7", "Preset", rows);
    expect(request.passes.map((p) => p.preset_id)).toEqual(["cameo5-htv", null]);
  });
});


describe("DeviceInfo.host", () => {
  it("distinguishes a cutter on this computer from one on a Cut Host", () => {
    const local: DeviceInfo = { ...aDevice(), host: null };
    const remote: DeviceInfo = { ...aDevice(), instance_id: "usb:sn:PI", host: "host-1" };
    expect(local.host).toBeNull();
    expect(remote.host).toBe("host-1");
  });
});

describe("effectiveSettings with an empty preset id", () => {
  // Codex's third gate: an empty id is a *named* preset, and truthiness treated it as absent — so
  // the dialog showed default speed and force while the cut path resolved the real entry. The
  // dialog and the machine have to agree about what the blade will do.
  it("resolves an empty id like any other, rather than showing defaults", () => {
    const presets = [{ id: "", name: "Nameless", machine_id: "cameo5",
                       settings: { speed: 7, force: 33, repeat_count: 2 }, builtin: false }];
    const row: PassVm = { key: "preset:", shapeCount: 1, enabled: true, presetId: "",
                          speed: null, force: null, repeatCount: null };
    expect(effectiveSettings(row, read(presets))).toEqual({ speed: 7, force: 33, repeatCount: 2 });
  });

  it("still treats a genuinely absent preset as absent", () => {
    const row: PassVm = { key: "no-preset", shapeCount: 1, enabled: true, presetId: null,
                          speed: null, force: null, repeatCount: null };
    expect(effectiveSettings(row, read())).toEqual({ speed: null, force: null, repeatCount: 1 });
  });
});
