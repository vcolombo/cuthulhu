<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Align and distribute — Implementation Plan

> **For agentic workers:** steps use checkbox (`- [ ]`) syntax. Each task is test-first: write the test, see it fail, implement, see it pass, commit.

**Goal:** six align buttons and two equal-gap distribute buttons. Each click commits every unit's translation as one `Delta` (one undo), through the hook's existing send path.

**Spec:** `docs/superpowers/specs/2026-10-07-align-distribute-design.md`, with these decisions confirmed on 2026-10-07: align to the selection's own bounds, no key object yet, and distribute by equal gaps.

**Revisions from gate 1** (the task bodies below stay as instructed):
- The queue key is per kind, axis and selection, not `align` (Task 4), so "Align top" does not replace a queued "Align left", nor a distribute an align.
- Distribute keeps the unit reaching furthest as the far end, not the last by start edge (Task 3).
- Units are counted from the scene and exclude ids beneath another selected id (`outermost`), so an empty Group or a Group's own child is not a unit. A unit spanning all the others is a frame that the rest are spaced inside, with margins equal to the gaps; distribute is disabled per axis, with the reason, when several units span the rest, the pieces do not fit the frame, they would pass each other, or a second frame sits inside the first (`distributeBlock`).

## Global constraints

- SPDX headers. Comments explain why. `// ponytail:` carries a ceiling and an upgrade path.
- `cargo test --locked`, with no new dependency, so `Cargo.lock` does not move.
- Any change to `apps/desktop/ui/src` rebuilds and commits `apps/desktop/ui/dist`.
- **`ipc-inventory.json` changes**: add `"commit_transforms": ["moves"]` by hand. The generating test needs the desktop crate, which needs WebKit, and this container cannot build that. CI's `rust` job is the check.
- Gate 1 (in-harness review on `git diff <base>...<head>`, with the test-coverage reviewer named) comes back clean before the PR is opened.

---

### Task 1: `transform_each` in `document`

- [ ] Tests in `crates/document/src/commands.rs`:
  - two entries → one `Delta`; applying it moves both, and its inverse restores both;
  - an unknown id in any entry → `Err`, and the document is unchanged;
  - a later entry sees an earlier entry's effect (an id moved twice composes);
  - a node whose ancestor is selected in another entry is moved once.
- [ ] Implement `pub fn transform_each(doc: &Document, moves: &[(Vec<NodeId>, Affine)]) -> Result<Delta, CmdError>`:
  - work on a scratch clone of the document;
  - run `transform_nodes` for each entry and apply its delta to the scratch, so later entries see earlier ones;
  - suppress a node whose ancestor any entry selects, across the whole batch;
  - return the concatenated ops, or the first error.
  - An empty `moves` is `Err(EmptySelection)`, matching `transform_nodes`.
- [ ] `cargo test -p document --locked`; commit.

### Task 2: the IPC command

- [ ] Add these, mirroring `commit_transform` exactly:
  - `state.rs`: `commit_transforms(moves)`;
  - `ipc.rs`: `#[tauri::command] commit_transforms(state, moves: Vec<TransformMove>)`, where `TransformMove { ids: Vec<NodeId>, m: Affine }` is serde-derived, with logic kept in `state.rs`;
  - `main.rs`: register it in `generate_handler!`;
  - `ipc-inventory.json`: `"commit_transforms": ["moves"]`, in the file's existing order.
- [ ] `ui/src/ipc.ts`: `commitTransforms({ moves })`.
- [ ] The e2e fake gains `commit_transforms`:
  - it reuses `commit_transform`'s mirrored maths per entry, in order;
  - it is all-or-nothing on unknown ids;
  - it records each entry to `__commitTransforms`, tagged with a batch number, so tests can see that one click is one batch.
- [ ] Commit. The Rust side is verified in CI; locally, `cargo test -p document` covers the logic.

### Task 3: the maths, `ui/src/interaction/align.ts`

- [ ] `align.test.ts`, test-first:
  - `alignMoves(units, mode, artboard)` for each of the six modes, with two or more units against the selection bounds;
  - one unit against the artboard;
  - zero moves are omitted, and moves are translations on one axis only;
  - `distributeMoves(units, axis)` with three or more units keeps the outer two and equalises the gaps;
  - the overlapping case gives negative gaps;
  - document order breaks ties;
  - fewer than three units gives `[]`.
  - A unit is `{ ids, bounds }`, and a Group unit's bounds cover all its shapes.
- [ ] Implement. Commit.

### Task 4: hook, App and panel

- [ ] `send()` takes `moves: { ids, m }[]`:
  - the pending preview applies each move to `expand(ids)`;
  - the commit callback sends the list;
  - App's callback calls `commitTransforms`. A gesture and a field edit become a list of one.
- [ ] Add `transformEach(key, make: (scene) => moves)` beside `transformWith`. It shares the queue (key `align`) and drops an empty list.
- [ ] App builds the units: one per selected id, with bounds from `expand(id)` in the effective scene. It wires eight handlers to the panel.
- [ ] `PropertiesPanel.tsx`: two button rows with `aria-label`s. They are disabled with no selection (align) or fewer than three units (distribute).
- [ ] Playwright, test-first, the five tests from the spec. Then build, run the full vitest and e2e suites, and commit.

### Task 5: checklist

- [ ] `MANUAL-CHECKLIST.md`, *Align and distribute (unverified)*: six names aligned left cut in a straight column on the Cameo 5, and distributed letters weed with even gaps. Commit.

## Verification

`cargo test --locked -p document -p geometry -p fileio -p cutplan -p trace`; `npm --prefix apps/desktop/ui test`; build with `dist/` current; `npm --prefix apps/desktop/ui run e2e`; gate 1 on `git diff origin/main...HEAD`; open the PR; gate 2.
