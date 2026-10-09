<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Weed lines — Implementation Plan

> **For agentic workers:** steps use checkbox (`- [ ]`) syntax. Each task is test-first: write the test, see it fail, implement, see it pass, commit.

**Goal:** an optional weed border, plus clipped weed lines, generated for each pass from that pass's shapes. They are cut after the shapes, within the same pass, and go through `plan_cut` and preflight like any other geometry.

**Spec:** `docs/superpowers/specs/2026-10-09-weed-lines-design.md`, confirmed 2026-10-09. The options travel in the payloads and are not saved in the project.

## Global constraints

- SPDX headers. Comments explain why. `// ponytail:` carries a ceiling and an upgrade path. Use the vocabulary in `CONTEXT.md`.
- `cargo test --locked`, with no new dependency, so `Cargo.lock` does not move.
- This container cannot build `desktop` (WebKit) or `cli` (libudev). Their Rust is checked by CI's `rust` job. Keep logic in `cutplan`, where it can be tested here, so those crates only pass values through.
- **`ipc-inventory.json`**: add `weed` to `plan_cut`, `travel_for_order` and `cut` by hand. Its generating test needs the desktop crate, so CI is the check.
- Any change to `apps/desktop/ui/src` rebuilds and commits `apps/desktop/ui/dist`. The UI build also typechecks e2e, which has no Node types.
- Gate 1 (in-harness review on `git diff <base>...<head>`, with the test-coverage reviewer named) comes back clean before the PR is opened.

---

### Task 1: `cutplan::weed`, the geometry

- [ ] Types: `WeedLines { None, Horizontal, Vertical, Both }` and `WeedOptions { margin_mm, lines, spacing_mm, clearance_mm }`, with serde shapes the UI can send as JSON. Add `WEED_RANGES` (margin 0.5–50, spacing 5–500, clearance 0.2–20, all in mm), plus `WeedOptions::validate() -> Result<(), WeedError>`, which also refuses clearance ≥ margin. `WeedError` is a sentence naming the field.
- [ ] `pub fn weed_geometry(shapes: &[PlannedShape], opts: &WeedOptions) -> Result<Vec<Polyline>, WeedError>` returns the lines first and the border last. If there is no geometry, or the bounds are not finite, it returns nothing; preflight then names the shape. It refuses a pass whose lines would exceed the line, work or piece caps (`MAX_WEED_LINES`, `MAX_WEED_WORK`, `MAX_WEED_PIECES`; added in gate 1).
- [ ] Tests:
  - the border is the bounds plus the margin;
  - lines are spaced from the border edge;
  - every piece keeps the clearance, checked by brute-force point-to-segment distance;
  - no piece inside a filled shape, but a piece inside a hole stays;
  - pieces under 2 mm are dropped;
  - an open path (no fill) blocks only by clearance;
  - every range edge, and clearance ≥ margin, is refused.
- [ ] `cargo test -p cutplan --locked`; commit.

### Task 2: weed in passes, `plan_cut` and preflight

- [ ] Add `DocumentPass::weed: Vec<Polyline>`, with `#[serde(default)]`; existing literals get `weed: vec![]`.
- [ ] Add `plan_passes_for(doc, grouping, weed: Option<&WeedOptions>) -> Result<DocumentPasses, PlanError>`. It validates, then fills each pass's `weed`. `PlanError::Weed(WeedError)` carries a refusal.
- [ ] `travel_moves` visits the weed polylines after a pass's shapes, one polyline per stop.
- [ ] In `plan_cut`, each `Job`'s polylines are the shapes followed by the weed.
- [ ] Preflight:
  - rules 2 and 3 scan the weed and refuse with `WeedGeometry { pass }`;
  - rule 4 refuses with `WeedOutOfBounds { pass, bounds }`, code `out_of_bounds`, and the sentence says to shrink the margin or move the design;
  - rule 7 counts weed points;
  - rule 1 still counts shapes only.
- [ ] Tests for each point above; commit.

### Task 3: desktop and CLI pass-through

- [ ] `plan_cut`, `travel_for_order` and `CutRequest` take `weed: Option<WeedOptions>`. All three call `plan_passes_for`.
- [ ] `PlanCutPassSummary.weed: Vec<Vec<[f64; 2]>>`.
- [ ] `settings_ranges` gains `weed: WEED_RANGES`.
- [ ] Update `ipc-inventory.json` by hand.
- [ ] Desktop tests: a bordered plan's summary carries the border; travel reaches it; a `CutRequest` with weed cuts it.
- [ ] CLI: add `--weed-margin`, `--weed-lines`, `--weed-spacing` and `--weed-clearance`, which build `WeedOptions` (defaults from the spec) and call `plan_passes_for`. Test the flag parsing.
- [ ] Commit; CI is the check.

### Task 4: UI

- [ ] In `ipc.ts`, add the `WeedOptions` type and the `weed` argument on the three calls.
- [ ] Add the summary's `weed` and the ranges' `weed`.
- [ ] Viewmodel: a pure `weedDraft` → `WeedOptions | null` plus field errors, from the ranges. Unit tests.
- [ ] Cut dialog:
  - a Weeding group with Border, margin, Lines, spacing and clearance;
  - the options are held with the plan, as `grouping` is, and a change replans;
  - an invalid field disables Cut;
  - App holds the last options for the session.
- [ ] `CutPreview` draws the weed in the pass's colour at lower alpha and counts the weed paths.
- [ ] The e2e fake mirrors `weed_geometry` for the fixtures. Specs:
  - the border shows in the preview;
  - Lines is disabled without Border;
  - a bad margin disables Cut;
  - the cut request carries the options;
  - reopening the dialog keeps them.
- [ ] Build, unit and e2e; commit with `dist/`.

### Task 5: checklist and close-out

- [ ] Add `MANUAL-CHECKLIST.md` entries for the Cameo 5 Alpha and the Puma IV: the border is cut last, lines never nick the design, and the strips weed.
- [ ] Run gate 1, then open the PR (it refers to #220 but does not close it).
