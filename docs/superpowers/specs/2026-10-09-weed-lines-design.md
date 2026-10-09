<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Weed lines — design

Date: 2026-10-09

The fourth step of the editor-first push, after the viewport and handles (#298), snapping (#300)
and align/distribute (#301). Cut a decal and every scrap of vinyl around it has to be peeled
off in one sheet. A **weed border** cuts the job free of the roll, and **weed lines** cut the waste
inside it into strips that lift one at a time. Silhouette Studio and Cricut Design Space both
offer this. Operators expect it, and without it they draw a rectangle by hand for every job.

This is the first slice of #220. The full issue rests on #60 (non-destructive media layout),
#159 (path offset) and #222 (shared line-fill engine), and #60 alone is weeks of work. This slice
ships the part an operator uses every day, keeps to the one planning path, and leaves #220 open
for the rest. The cut-workflow spec deferred weed lines because "they need polygon offsetting"
(`2026-07-24-cut-workflow-design.md`). The border here is a rectangle, and clearance is measured
against the flattened outline, so neither needs offsetting.

## Where it starts

From `main` at `2057e75`:

- `plan_passes_with(doc, grouping)` walks the document into `DocumentPasses`: one `DocumentPass`
  per key, each a list of `PlannedShape { node_id, polylines }` in world mm
  (`crates/cutplan/src/passes.rs`).
- `plan_cut(planned, profile, caps, opts)` selects passes, runs `preflight` on them and builds one
  `Job` per pass from its shapes' polylines (`crates/cutplan/src/plan.rs`).
- The desktop calls `plan_passes_with` in three places: `plan_cut_response` (the dialog's
  plan and preview), `travel_for_order` (the preview after a reorder) and `prepare_cut` (the cut).
  The `Grouping` travels in all three payloads, because a mode kept in `AppState` could change
  between round trips (`apps/desktop/src/device.rs`).
- `preflight` checks the shapes' polylines: finite, two points or more, inside the machine's
  area, output size (`crates/cutplan/src/preflight.rs`). Errors that point at geometry name a
  `NodeId`.

## Decisions

- **Weeding belongs to a pass, not to the document.** A pass is one sheet of material: under
  colour grouping, the red pass is cut on red vinyl and the blue pass on blue. Each needs its own
  border around its own shapes. A border around everything would leave a red sheet with a frame
  sized for the blue design. So each pass gets weed geometry computed from that pass's shapes
  only, cut in that pass with that pass's settings.
- **What is generated:**
  - **Border:** the axis-aligned bounding box of the pass's polylines, grown by the margin on
    every side. One closed rectangle.
    (`// ponytail:` a box, not a contour. Ceiling: an L-shaped or diagonal design gets a lot of
    waste inside its box. Upgrade: a contour border from #159's path offset.)
  - **Lines:** none, horizontal, vertical, or both. Lines sit at the spacing, measured from the
    border's top (or left) edge, and run from border edge to border edge. Each line is clipped
    twice:
    1. **Clearance.** Removed wherever it comes within the clearance of any of the pass's
       polylines. For a horizontal line at `y`, every segment's part inside the band
       `[y - clearance, y + clearance]` blocks its x-range, widened by the clearance on each side.
       That blocks a superset of the points truly within the clearance, so a line never passes
       closer than that. Vertical lines are the same with the axes swapped.
    2. **Inside a shape.** Each piece left over is tested at its midpoint against every closed
       outline of the pass, one shape at a time, by the non-zero winding rule, which is how the
       app fills (`geometry::boolean`, SVG's default). A piece inside a shape's filled area is
       dropped, since cutting it would slice the decal; that includes a pentagram's centre and
       the overlap of two subpaths. A piece inside a hole (the counter of an O, which winds the
       other way) stays: that waste has to come out too. (Revised in gate 1 from even-odd, which
       cut lines through those filled areas.) A hole drawn as a separate node is a disc of its
       own, cut as its own piece, so the waste between them is not weeded.

    Pieces shorter than `MIN_WEED_PIECE_MM` (2 mm) are dropped. A nick that short weeds nothing
    and costs a blade lift. A pass that would need more than `MAX_WEED_LINES` (10 000) lines on
    one axis is refused before any is built: lines are generated before preflight, under the
    document lock, and a stray scale would otherwise build millions (added in gate 1).
    (`// ponytail:` straight lines at a fixed spacing. Ceiling: no diagonal lines, and no lines
    that bend around shapes. Upgrade: #222's line-fill engine.)
- **Cut order within a pass:** the pass's shapes first, then the lines, then the border last.
  Cutting the border early frees the sheet while the blade still has detail to cut, and a sheet
  that has shifted ruins the detail.
- **Options are one value per cut:** `WeedOptions { margin_mm, lines, spacing_mm,
  clearance_mm }`, or none for no weeding. Every pass in the cut gets the same options.
  (`// ponytail:` no per-pass options. Ceiling: you can't border the red sheet and leave the blue
  one bare in a single cut. Upgrade: a weed field on the dialog's pass row.)
- **Ranges live in `cutplan`, once.** A `WEED_RANGES` constant sits beside `SETTINGS_RANGES` and
  reaches the UI on the same `settings_ranges` response, the same arrangement as the speed and
  force ranges.
  - Margin: 0.5–50 mm.
  - Spacing: 5–500 mm.
  - Clearance: 0.2–20 mm, and strictly less than the margin, or the border would sit inside the
    clearance zone.

  A value out of range is refused with a sentence naming the field, never clamped.
- **Generated in `cutplan`, before `plan_cut`.** A new `cutplan::weed` module holds the geometry.
  A new `plan_passes_for(doc, grouping, weed: Option<&WeedOptions>)` returns `DocumentPasses`
  whose passes carry their weed polylines in a new field, `DocumentPass::weed: Vec<Polyline>`.
  All three desktop paths and the CLI call it, so the preview, the travel and the cut cannot be
  built from different weed geometry. `plan_passes_with` stays as it is, as the same call with
  no weeding.
  - A separate field, not extra `PlannedShape`s: a `PlannedShape` is a document node by
    definition (`node_id`), and the dialog counts shapes and anchors order badges on them.
  - `plan_cut` builds each `Job` from shapes, then weed, in the order above. Weeding is not a
    way around the chokepoint; it is more geometry going through it.
- **Preflight checks weed geometry like any other.** Rules 2–4 and output size scan
  `pass.weed` too. The geometry errors gain a variant that names a pass's weeding instead of a
  node: `OutOfBounds` becomes "the weed border for pass color:ff0000ff extends outside the
  machine's area…". This is the refusal an operator will actually meet, with a design near the
  mat's edge, so the sentence also says to shrink the margin or move the design.
  `NothingToCut` keeps counting shapes only, since a border around nothing is not a job.
- **The options travel in the payload, like `Grouping`.** `plan_cut`, `travel_for_order` and
  `cut` each take `weed: WeedOptions | null`. The dialog keeps the options with the plan whose
  rows they produced, just as it keeps the grouping. Rows planned with one border and cut with
  another would send geometry nobody previewed, and the revision check guards only the
  document. Changing an option replans. `apps/desktop/ipc-inventory.json` is regenerated.
- **Not saved in the project.** This is a change from the plan as first proposed. Stored in the
  `Document`, every tweak in the dialog would be an undoable edit that also makes the open
  plan stale, and the file format would need a version 3 and a migration. Grouping isn't saved
  either; the dialog owns both. The options live in App state for the session, so reopening the
  dialog keeps the last ones, and they start off (no weeding) in a new session.
  (`// ponytail:` per session. Ceiling: an operator who always borders at 5 mm sets it once per
  launch. Upgrade: remember the last options in the app's config, beside `presets.json`.)
- **Preview.** `PlanCutPassSummary` gains `weed: Vec<Vec<[f64; 2]>>`. `CutPreview` draws it in
  the pass's colour, at the cut line's width with a lighter alpha, so it reads as "also cut" but
  distinct from the design. The summary line counts weed paths. Travel includes the weed
  geometry in cut order, so the dashed moves show the blade going to the lines and then to the
  border.
- **Dialog controls**, in one "Weeding" group under the pass list:
  - a "Border" checkbox (off by default) with a margin field;
  - a "Lines" select (None, Horizontal, Vertical, Both) with spacing and clearance fields, enabled
    only while Border is on.

  Lines without a border are not offered: their ends would stop in the open sheet. Fields show
  their range from `settings_ranges`. An out-of-range value marks the field and disables Cut, as
  the settings fields already do. A weed edit replans but keeps the operator's pass order and
  settings, since it changes no pass (added in gate 1).
- **CLI:** `cuthulhu cut --weed-margin MM [--weed-lines h|v|both] [--weed-spacing MM]
  [--weed-clearance MM]`. Without `--weed-margin`, there is no weeding. A flag that would do
  nothing is refused rather than ignored: line flags without `--weed-margin`, and spacing or
  clearance without `--weed-lines`. It is the same
  `plan_passes_for` call, so `--dry-run` reports the weed geometry too.

## Defaults

Margin 3 mm, lines None, spacing 25 mm, clearance 1.5 mm. These are starting values, not facts
about a machine, so they cite nothing. The hardware checks below are where they get tuned.

## Testing

- **`cutplan::weed` unit tests:**
  - the border is the pass's bounds grown by the margin;
  - one border per pass, each sized to its own pass;
  - every line piece keeps at least the clearance from every polyline (checked by brute-force
    distance against the flattened segments, not by the band arithmetic under test);
  - no piece inside a filled shape;
  - a piece inside a hole stays;
  - pieces shorter than 2 mm are dropped;
  - the order is shapes, lines, border;
  - each range edge, and clearance ≥ margin, is refused with its sentence.
- **`plan_cut` / preflight:**
  - weed geometry is in the `Job`, in order;
  - a border past the mat is refused with the weeding variant;
  - `allow_out_of_bounds` lets it through;
  - output size counts weed points.
- **Desktop `state`/`device` tests:**
  - the three payloads carry `weed`;
  - a cut sent with different weed options from the preview's still cuts what it was sent.

  The second case is pinned because it is the reason the options travel.
- **e2e:** the fake mirrors the border exactly, and lines for the axis-aligned rectangles the
  fixtures use. The specs cover:
  - turning on Border draws it in the preview;
  - Lines stays disabled without Border;
  - an out-of-range margin disables Cut;
  - the cut request carries the options;
  - reopening the dialog keeps them.
- **`MANUAL-CHECKLIST.md`:** a bordered, lined job cut on the Cameo 5 Alpha and on the Puma IV.
  Check that the border is cut last, no line nicks the design, and the strips weed off.

## Out of scope

- #60's media layout, Materialize, and any weed geometry written into the document.
- A contour (offset) border (#159), diagonal or fill-pattern lines (#222).
- Per-pass weed options, and saving options in the project or config.
- Weeding for print-and-cut jobs. Registration marks are a later sub-project, and a border would
  have to keep clear of them.
