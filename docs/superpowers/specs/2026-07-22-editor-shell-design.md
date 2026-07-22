# Sub-project 3: Editor shell — design

The design canvas and editing foundation for cuthulhu. Third sub-project after
the protocol spike (SP1) and drivers + CLI (SP2). Governed by the master design
spec (`2026-07-21-cuthulhu-design.md`); this document specs SP3 only.

**Goal:** a fast, modern vector editor that opens/imports SVG, lets you place
and arrange shapes and text on the machine's real cuttable area, and saves to an
open project format — proving cuthulhu's UX/performance thesis before any cutting
UI exists. No cutting in SP3 (that is SP4).

## Decisions (settled in brainstorming)

- **Stack:** Tauri shell, TypeScript + React UI, a custom **scene-graph** renderer over Canvas2D (dirty-rect redraw), IPC to the Rust core. WebGL is a later swap behind the renderer interface — Canvas2D first, measure, upgrade only if perf demands.
- **Document model: hybrid.** Rust owns the authoritative document, geometry, file I/O, and undo/redo (single source of truth, headless-testable). TS holds only ephemeral interaction state (live drag/marquee/handles) for 60fps feedback and commits on mouse-up.
- **Import scope V1: SVG only.** PDF/DXF deferred to a later pass. Project save/load is always included.
- **Layout: conventional** (left tool rail / right layers + properties / top bar), so users migrating from other tools don't relearn. Node/Bézier editing deferred post-V1.
- **Design language: "Workbench"** — dark-first, token-driven precision workspace (see below).

## Architecture

Rust workspace (bootstrapped by SP2; SP3 adds the `document` crate and the app):

```
crates/
  geometry/    [SP2] booleans, offsets, transforms, text-to-path
  fileio/      [SP2] SVG import; project save/load (zip container)
  document/    [SP3 NEW] authoritative editable model — SP3's core deliverable
apps/desktop/
  (Rust)       Tauri main; IPC commands calling document / geometry / fileio
  ui/          [SP3 NEW] React + TS: scene-graph renderer, interaction, panels, Workbench tokens
```

### `document` crate (new — the heart of SP3)

Owns the authoritative editable state, headless and unit-tested:

- **Scene tree:** layers → groups → shapes (path, rect, ellipse, text). Each node has an id, transform, and stroke/fill style.
- **Machine artboard:** the cuttable-area bounds, driven by a machine profile.
- **Command + undo/redo stack:** every edit is a command; one committed gesture/command = one undo entry.
- **Delta emission:** each applied command returns a scene delta for the UI to render.

The UI reaches geometry and the document only through Tauri IPC. The Rust core is fully testable without the UI.

### IPC boundary

- **TS → Rust commands:** `new_doc`, `open_svg`, `apply_command`, `commit_transform(ids, matrix)`, `boolean_op`, `add_text`, `add_primitive`, `reorder_layer`, `group`/`ungroup`, `delete`, `align`, `undo`, `redo`, `save_project(path)`, `load_project(path)`, `set_machine(id)`.
- **Rust → TS:** first load returns a full **snapshot**; every edit returns a **delta** = `[{op: add|update|remove, nodeId, patch}]`. Deliberately boring.

### Renderer

The TS scene-graph mirrors the Rust document as a render tree; dirty-rect redraw keeps large designs at 60fps. A `Renderer` interface hides Canvas2D so a WebGL backend can replace it later without touching interaction code.

## UI structure

```
┌─────────────────────────────────────────────────────────────┐
│ TOP BAR: file · undo/redo · [machine ▾] · zoom · units       │
├──────┬────────────────────────────────────────┬─────────────┤
│ TOOL │                                        │  LAYERS      │
│ RAIL │        CANVAS = machine artboard       │  (tree)      │
│      │        (cuttable area + grid)          ├─────────────┤
│ sel  │                                        │  PROPERTIES  │
│ txt  │                                        │  X Y W H ∠   │
│ rect │                                        │  align/dist  │
│ ell  │                                        │  stroke/fill │
│ bool │                                        │             │
├──────┴────────────────────────────────────────┴─────────────┤
│ STATUS: machine · zoom % · cursor x,y · selection count      │
└─────────────────────────────────────────────────────────────┘
```

**Canvas = the machine's real cuttable area** (from the machine profile). Switching machine in the top bar resizes the artboard — the honest-mat differentiator versus a fake fixed mat.

### V1 tool set

- **Select/transform** — move, scale (handles), rotate; marquee + multi-select; group/ungroup
- **Text** — system fonts; text-to-path performed Rust-side at cut/export time
- **Primitives** — rectangle, ellipse
- **Boolean** — union / subtract / intersect / exclude (toolbar actions on selection → `geometry`)
- **Align/distribute**, **import SVG + place**, **save/load project**, **zoom/pan**, **undo/redo**

**Deferred (later sub-projects, not SP3):** node/Bézier editing, pen/freehand, cut send (SP4), trace (SP5), print & cut (SP6).

**Properties model:** for a cutting tool, *stroke = the cut line*. V1 surfaces stroke (cut path) prominently; fill is preview-only. Nothing cuts until SP4, but the mental model is established here.

## Design language — "Workbench"

Dark-first, token-driven; chrome recedes, the artwork carries the color. Light theme ships too. All colors are CSS-variable tokens (shared with the canvas background/grid).

| Token | Dark | Use |
|-------|------|-----|
| `workspace` | `#17171A` | canvas backdrop |
| `panel` / `border` | `#1F1F23` / `#2E2E34` | chrome surfaces |
| `text` / `muted` | `#E7E7EA` / `#9A9AA2` | text |
| `accent` | `#22D3EE` (cyan) | selection, handles, active tool, primary buttons |
| `cut` | `#FF4D4D` (red) | cut-path preview — **reserved, unused in SP3** |
| `ready` | `#34D399` (green) | machine-ready / success |

Two-signal rule: **cyan = what you selected, red = what the blade does.** Never conflate them.

Density: pro-tool compact (~30px rows). **Tabular numerals** with **drag-scrub** on the X/Y/W/H/∠ fields. One clean sans (Inter or system-UI) for chrome. Thin 20px line icons. Motion 100–150ms, no bounce. Brand mark (Lovecraft tentacle/blade pun) confined to a small corner glyph — never intrudes on the workspace.

## Data flow

**Direct manipulation (drag/scale/rotate) — the 60fps path:**
1. `pointerdown` → TS hit-tests its scene-graph → sets selection locally.
2. `pointermove` → TS applies the transform matrix optimistically to selected render nodes → dirty-rect redraw + live coords. **No IPC.**
3. `pointerup` → one `commit_transform(ids, matrix)` → Rust updates the authoritative model, pushes one undo entry, returns a delta → TS reconciles (normally a no-op).

**Structural edits (boolean / add text / primitive / group / reorder / delete):** TS → IPC → Rust mutates the doc (using `geometry`) → pushes undo → returns delta → TS patches the affected nodes.

**Undo/redo:** single authoritative stack in `document`; TS holds no history. Ephemeral drag state never enters it — one gesture = one undo step.

**Import SVG:** path → `open_svg` → Rust `fileio` (usvg) parses to doc nodes → full snapshot → TS render tree.

**Save/load project** (open, documented container):
- `save_project` → zip: `design.svg` (canonical geometry) + `manifest.json` (machine profile, layer state, units, app version) + `assets/` (images, later).
- `load_project` → Rust reads zip → doc → snapshot → TS.

**Machine switch:** `set_machine(id)` → Rust resets artboard bounds → delta → TS resizes. Profiles come from `driver-core` (SP2); SP3 stubs two profiles (Cameo 5, Puma IV) so the editor stands alone before drivers land.

## Error handling

No silent failures — consistent with the master spec.

- **IPC commands** return typed `Result`. Failure → non-blocking toast/inline message; the canvas never crashes. A failed command mutates nothing and emits no delta, so the doc stays consistent.
- **SVG import** → `ImportReport { imported, skipped[] }`: parse what is valid, report unsupported elements rather than silently dropping them.
- **Geometry ops** (degenerate/self-intersecting paths) → `Result`; UI reports "operation failed," doc untouched (no partial mutation).
- **Save** is atomic (temp file + rename); a crash mid-save cannot corrupt a project. **Load** validates the `manifest.json` version; a newer-than-known version warns rather than misreads.
- **Reconcile mismatch** (optimistic TS transform vs Rust delta, rare) → Rust is authoritative, TS snaps to it; log if divergence exceeds an epsilon.

## Testing

- **`document` (Rust):** unit tests for command apply/undo/redo invariants, scene-tree ops, transform math, artboard bounds. **Property test:** apply → undo restores the prior state exactly.
- **`fileio`:** golden SVG→doc; **round-trip** save→load yields an identical doc.
- **`geometry` (SP2, reused):** golden tests for booleans/offsets/text-to-path.
- **TS:** unit tests only for interaction-critical *pure* logic — hit-testing, the optimistic transform matrix, delta reconciliation. UI stays thin so real logic lives in Rust where it is tested.
- **One thin Tauri E2E smoke** (launch → new doc → add rect → save → reload) plus a manual visual/interaction checklist. No heavy UI test framework.

## Definition of done (demoable)

Launch the app → the machine artboard shows (pick the Cameo 5 or Puma IV stub) →
import an SVG, add text, draw a rectangle → arrange with move/scale/rotate,
boolean, and align → save the project → reload it identical → the canvas stays
60fps on a large SVG. No cutting (SP4).

## Dependencies

Builds on **SP2's Rust workspace** — `geometry`, `fileio`, and `driver-core`
machine profiles. Per the build order SP2 lands first; SP3 adds the `document`
crate, the Tauri app, and the UI on top.

## Out of scope (SP3)

- Cut send / device control (SP4).
- Bitmap trace (SP5).
- Print & cut / registration (SP6).
- Node/Bézier editing, pen/freehand tools.
- PDF and DXF import.
- WebGL renderer backend (interface reserved; Canvas2D ships).
