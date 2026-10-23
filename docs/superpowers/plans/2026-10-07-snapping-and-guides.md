<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Snapping and smart guides — Implementation Plan

> **For agentic workers:** steps use checkbox (`- [ ]`) syntax for tracking. Each task is test-first: write the test, see it fail, implement, see it pass, commit.

**Goal:** move and axis-aligned scale gestures snap to the artboard's and other shapes' edge and centre lines within 6 CSS px, with guides drawn while snapped and ⌘/Ctrl held to drag free.

**Architecture:** a pure `interaction/snap.ts` gathers per-axis target lines once per gesture and adjusts the gesture's *pointer* before `gestureMatrix` runs. Preview, commit, Shift and Alt therefore stay unchanged. The hook keeps the targets and the current guides on the transform gesture. The renderer draws the guides from the overlay.

**Spec:** `docs/superpowers/specs/2026-10-07-snapping-and-guides-design.md`.
> **Implementation and review overturned some details after this plan was written. The task bodies
> below still state the originals and are left as they were**, as with the earlier plans: this
> file records the instructions the implementation was given. What shipped:
>
> 1. **Task 3, test 3 expects `a` ≈ 3, not 2.** The right edge goes from 10 to 30 mm with the left
>    edge at 0, so the 10 mm rect becomes 30 mm wide. Unsnapped, it would be 2.96 (CodeRabbit on
>    #300).
> 2. **Guides are kept only for lines the landed box touches, and stretched over it.** `keepLanded`
>    runs after `gestureMatrix`. Each guide carries its `axis` rather than having it inferred from
>    its ends, and a Shift move holds its locked axis at the start (gate-1 review).
> 3. **Task 3 grew from three e2e tests to seven**: press-and-drag, the artboard's centre line, a
>    Group not snapping to its own shape, and Alt edge scaling (gate-1 review).

## Global constraints

- SPDX header on new files. Comments explain why. `// ponytail:` carries a ceiling and an upgrade path.
- Every task that touches `apps/desktop/ui/src` rebuilds and commits `apps/desktop/ui/dist`.
- No IPC change: `ipc-inventory.json` and `Cargo.lock` must not move.
- Review: gate 1 (in-harness agents on `git diff <base>...<head>`, test-coverage reviewer by name) comes back clean before the PR is opened; gate 2 (installed bots) comes back clean after. `CLAUDE.md`, *PR review*.

## File structure

| File | Change |
|---|---|
| `ui/src/interaction/snap.ts` | **New.** `SNAP_PX`, `Line`, `Targets`, `Guide`, `snapTargets`, `snapMove`, `snapScale`, `boxBounds`. |
| `ui/src/interaction/snap.test.ts` | **New.** |
| `ui/src/render/Renderer.ts` | `Overlay` gains `guides: Guide[]`. |
| `ui/src/render/Canvas2DRenderer.ts` | Draws guides, 1 px, `--guide`. |
| `ui/src/tokens.css` | `--guide`. |
| `ui/src/interaction/useCanvasInteraction.ts` | Targets on press; `follow()` snaps the pointer; guides to the overlay; ⌘/Ctrl bypass. |
| `ui/e2e/smoke.spec.ts` | Three tests. The fake already seeds both the two rects and the Group. |
| `apps/desktop/MANUAL-CHECKLIST.md` | A *Snapping* section. |

---

### Task 1: The snapping maths

- [ ] **Step 1: Write the failing tests** in `snap.test.ts`. Use these fixtures: rect A at (0, 0, 10, 10), rect B at (30, 0, 10, 10), artboard (0, 0, 330, 3000), and tolerance 1 mm.
  - **Targets:**
    - `snapTargets` excludes the given ids;
    - it includes the artboard's six lines;
    - its lines are sorted by `v`.
  - **Move:**
    - moving A by (19.6, 0) snaps x by +0.4 (A's right edge meets B's left) and leaves y alone;
    - moving it by (17, 0) does not snap x (the nearest line is 2 mm off);
    - when two candidates are equally close, the centre line wins;
    - with `lockShift` and a mostly-x drag, only x snaps, and the y delta is untouched.
  - **Scale:**
    - the `e` handle of A dragged +19.6 snaps to 30;
    - the `n` handle does not touch x;
    - a box whose frame is rotated 30° returns the pointer unchanged and no guides.
  - **Guides:**
    - the x snap from the move case yields one vertical segment at x = 30, spanning y from 0 to 10, which covers A and B.
  - **Tolerance:** the same 6 px reaches four times farther in mm at a quarter of the zoom; test `SNAP_PX / scale` at two scales.
- [ ] **Step 2: Run, expect FAIL** (`npm --prefix apps/desktop/ui test -- snap`).
- [ ] **Step 3: Implement `snap.ts`:**
  - `boxBounds(box)` is `transformBounds(box.frame, {0, 0, w, h})`.
  - `snapTargets` emits, for every non-excluded scene node and the artboard, x lines at min, centre and max (each `lo..hi` = the box's y extent), and y lines likewise. It sorts by `v`.
  - `nearest(lines, v, tol)` binary-searches the lower bound, checks both neighbours, and returns every line at the nearest coordinate within `tol` (lower `v` wins a distance tie), or null.
  - `snapAxis(cands, lines, tol)` tries candidates in priority order (centre, min, max) and keeps the smallest distance. A later candidate replaces an earlier one only if strictly nearer.
  - `snapMove(box, start, cur, t, tol, shift)`:
    - the lock comes from the *raw* drag (`|dx| >= |dy|` frees x), matching `gesture.ts`'s `moveMatrix`;
    - it snaps the moved box's lines on the free axes;
    - it returns `cur` plus the deltas, and one guide per snapped axis spanning the moved box and the matched lines.
  - `snapScale(box, h, start, cur, t, tol)`:
    - only when the frame is axis-aligned (`|b|, |c| < 1e-9`, `a, d > 0`);
    - on each axis the handle moves (`HANDLE_UNIT` ≠ 0.5), it snaps the handle's dragged world coordinate;
    - it returns the adjusted pointer and guides;
    - otherwise it returns `cur` with no guides, carrying the spec's rotated-shape `// ponytail:`.
- [ ] **Step 4: PASS, build, commit:** "Add the snapping maths as a pure module, since pointer-level snapping keeps preview and commit untouched".

### Task 2: Guides on screen

- [ ] Add `guides: Guide[]` to `Overlay`. Every `setOverlay` call site passes `guides: []` until Task 3.
- [ ] The renderer draws the guides before the box, so the box and handles they line up stay on top. Each is a screen-space line between the view-mapped endpoints, 1 px, in `--guide` (fallback `#F472B6`).
- [ ] `tokens.css` gets `--guide` in its `:root` (there is one theme today), with a comment: "smart guides — distinct from selection cyan and cut red".
- [ ] Build (no unit test: the renderer has none, by design), commit: "Draw smart guides in their own colour, since cyan means selection and red means cut".

### Task 3: Wire it into gestures

- [ ] **Step 1: Write the failing e2e tests**, with `seedTwoColorRects` and `seedGroup` together, zoomed in at (20, 5):
  1. Drag the red rect's centre by +19.6 mm on x. The commit's `e` is close to 20, because its right edge snapped to the Group's rect at 30.
  2. The same drag with `Control` held commits `e` close to 19.6.
  3. Drag the red rect's `e` handle at (10, 5) by +19.6 mm. The commit's `a` is close to 2 and its `e` close to 0.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement in the hook:**
  - The transform `Gesture` gains `targets: Targets` and `guides: Guide[]`.
  - On press, for move and scale gestures: `targets = snapTargets(scene, shapes, artboard)`, with `artboard` added to `latest`.
  - A `snapOff` ref is set from `e.metaKey || e.ctrlKey` on press and on every move.
  - In `follow()`, for transform gestures other than rotate, and unless `snapOff`, compute `tol = SNAP_PX / viewRef.current.scale`. Pass the world pointer through `snapMove` (with `lastMods.shift`) or `snapScale`, then into `gestureMatrix`. Store the guides.
  - `repaint` puts the transform gesture's guides on the overlay, and `[]` otherwise.
  - Guides clear on release, because the gesture becomes null.
- [ ] **Step 4: PASS (×3 for stability), full vitest and Playwright, build, commit:** "Snap moves and axis-aligned scales to artboard and shape lines, since lining pieces up is most of placing them".

### Task 4: Hardware checks

- [ ] Add to `MANUAL-CHECKLIST.md` under *Snapping (spec 2026-10-07, unverified)*:
  - guides read clearly against the artboard and the workspace;
  - snapping feels sticky, not grabby, at fit and at 400%;
  - ⌘ mid-drag releases the snap;
  - a decal snapped to the artboard's left edge cuts flush with the mat edge on the Cameo 5.
- [ ] Commit: "List snapping's hardware checks, since feel and flush cuts are only judged on a device".

## Verification

`npm --prefix apps/desktop/ui test`; `npm --prefix apps/desktop/ui run build && git diff --exit-code apps/desktop/ui/dist`; `npm --prefix apps/desktop/ui run e2e`; then gate 1 on `git diff origin/main...HEAD` before opening the PR.
