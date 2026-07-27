<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Cuthulhu: GPLv3 desktop cutting software for vinyl cutters (Silhouette Cameo 5 Alpha over USB,
GCC Puma IV over HPGL/serial). A Rust workspace engine behind a Tauri + React shell, plus a CLI
that shares the same planning path.

`README.md` is stale on status — it claims there is no app and no GUI. Both exist
(`apps/desktop`, ten crates, a working cut workflow and trace). Trust the code and
`docs/superpowers/plans/` over the README's status block.

`tools/` is the frozen Python protocol spike (USB decoder, HPGL/GPGL square senders). It is
research tooling, not the product — do not grow it.

## Commands

```sh
cargo test --workspace --locked          # what CI runs; --locked is mandatory (see below)
cargo test -p cutplan preflight          # one crate, filtered by test-name substring
cargo run -p cli -- cut file.svg --dry-run

cd apps/desktop/ui
npm run dev                              # vite dev server on :5173
npm run build                            # tsc + vite build -> dist/ (must be committed)
npm test                                 # vitest run
npm test -- viewmodel                    # one file
npx playwright install chromium          # once per checkout; e2e cannot launch without it
npm run e2e                              # playwright; boots `npm run dev` itself

cargo tauri dev                          # from apps/desktop (cargo-tauri installed globally)
cargo tauri build

cd tools && python3 -m pytest            # spike tool tests
```

Two CI gates that fail on ordinary-looking commits:

- **`--locked`**: `Cargo.lock` is committed and CI refuses to rewrite it. Add a dependency →
  commit the lock in the same change.
- **`ui/dist` staleness**: `dist/` is committed because `tauri::generate_context!` reads
  `frontendDist` at Rust compile time, so the Rust CI job (no Node) needs it present. CI rebuilds
  and fails if the committed bundle differs. Touch `ui/src` → run
  `npm --prefix apps/desktop/ui run build` and commit `dist/`.

## Architecture

Dependency spine, strictly one-way:

```
geometry ──► document ──► cutplan ──► driver-core (traits only)
                │            │              ▲
                └── fileio   └── trace      │
                                       driver-registry ──► driver-silhouette, driver-hpgl
                                            ▲
                              crates/cli, apps/desktop  (the two binaries)
```

### The cut path — one chokepoint

`Document` → `plan_passes` → `DocumentPasses` → `plan_cut` → `CutPlan` → `cut_passes()` →
`DeviceManager::cut` → `Driver` bytes → `Transport`.

`cutplan::plan_cut` is the single chokepoint, and every path now goes through it. Everything that
can refuse a cut lives behind it: the stale-plan check (`doc_revision`), colour→pass matching, and
`preflight` (finite/in-bounds/non-degenerate geometry, settings within machine range, document
meant for the connected machine, output size). A caller hands over `DocumentPasses` plus a
`PlanOptions` and gets a `CutPlan` or a `CutError`. Do not add a way around it; extend it.

A plain `cuthulhu cut` (no `--by-color`) reaches it by giving every imported path a uniform stroke
(`pipeline::doc_from_svg_all_cuttable`), so `plan_passes` sees all the geometry and groups it into
exactly one `ColorPass`. That is the plain path saying explicitly what it always meant — everything
in the file, one pass — rather than arriving there by skipping the planner. It deliberately does
not change `plan_passes`' stroke rule; whether cuttability should follow the path or the stroke is
still open in issue #68.

A pass is not "enabled/disabled": a colour nobody lists in `PlanOptions::passes` is not cut.

### Devices

- `driver-core` holds *only* traits and shared types (`Driver`, `Transport`,
  `DeviceBackendFactory`, `Job`, `Settings`, `MachineProfile`, `MachineCaps`). It must never
  depend on a concrete driver.
- `driver-registry` is the single place mapping real hardware to drivers/transports. Machine ids
  (`cameo5`, `puma`) are bound to one constant each there, and a test pins them against each
  driver's own `MachineProfile::id` so the three copies cannot drift. Legacy ids
  (`cameo5_alpha`, `puma_iv`) are migrated on project load in `fileio::load_project`.
- `driver_core::manager::DeviceManager` owns a worker thread plus a bounded command channel; all
  transport access is serialized through it. It drives session framing (`session_begin` once,
  per-pass `encode_pass`/`pass_park`, `session_end` once) and a per-pass completion policy.
  `cancel` is cooperative/best-effort — a shared atomic checked between transmit chunks and ENQ
  polls, plus a queued `Command::Cancel` for a worker parked at `recv()`.
- **A caller is told about a cut through one value**: `CutStatus { phase, ended, actions, pass,
  sent, error }` (`driver-core/src/status.rs`). `phase` is what the machine is doing now; `ended`
  is how the last cut finished; `actions` says which of cut/cancel/resume/confirm are legal, so a
  caller renders controls from that rather than mapping phases back to permissions. The worker
  publishes it to shared memory before sending each event, so `status()` never blocks — and a
  finished worker reports a failure rather than a frozen value.
- The state machine behind it is **not public**. `DeviceState` is `pub(crate)`, enforced by rustc
  rather than convention. Three callers used to re-derive it across two languages; if you find
  yourself reconstructing "what is legal now" from anything other than `actions`, stop.
- `MockTransport` in `driver-core` is how device behaviour gets tested headless.

### Desktop app

- `apps/desktop/src/state.rs` carries the logic (one method per IPC command, delegating to
  `document`/`fileio`/`cutplan`); `ipc.rs` is a thin `#[tauri::command]` layer that only maps
  typed errors to `String`. Keep logic out of `ipc.rs`.
- `main.rs` runs the **event bridge**: the sole consumer of the device-event channel. It coalesces
  `Progress` to ≤10 Hz — that is about webview emit cost, not about the cut — and forwards
  everything else immediately. It keeps no state of its own; `get_device_state` and the
  window-close handler read `driver-core`'s published `CutStatus`, which never blocks.
- The UI reaches Rust only through `ui/src/ipc.ts`. Rendering is a hand-written Canvas2D renderer
  (`ui/src/render/`); each dialog splits into a pure `viewmodel.ts` (unit-tested) plus a thin
  `.tsx`. `e2e/smoke.spec.ts` installs an in-page fake Tauri backend that mirrors
  `Document::snapshot_json()` — change that JSON shape and the mock must change with it.

### Document and files

- Every edit is a `Delta` whose inverse is derivable; `Editor` keeps inverse-delta undo and
  forward-delta redo stacks, so undo is just applying the inverse. Multi-step gestures commit as
  one `Delta`.
- `cutplan::doc_revision(doc)` is what "stale plan" compares against — a cut planned against a
  document that has since changed is refused, not cut.
- Project file is a zip: `manifest.json` (the source of truth, `Document::snapshot_json()`) plus
  `design.svg` (a best-effort interchange copy; unsupported node kinds become comments). Saves
  are atomic — temp file in the destination directory, then rename.
- Material presets: builtins ship in `cutplan::presets`; user presets live in
  `<config_dir>/cuthulhu/presets.json` and the on-disk contract is *user entries only*
  (`builtin: false` forced on write, user entries shadow builtins by id).

## Conventions

- **`CONTEXT.md` is normative vocabulary.** It defines Document, Node, Delta, ColorPass,
  DocumentPasses, PassSelection, CutPlan, Preflight, Settings, MaterialPreset, MachineProfile,
  MachineCaps, Driver, Transport, Job, Pass — each with an explicit `_Avoid_` list (no "layer"
  for ColorPass, no "backend"/"plugin" for Driver, no "validation" for Preflight). Use these
  words in code, comments, commits, and issues.
- **SPDX header on every file**: `// SPDX-License-Identifier: GPL-3.0-or-later` (or the
  language's comment form — `<!-- ... -->` in Markdown, `#` in Python).
- **Comments explain why, not what.** The existing ones document the constraint or the trap that
  forced the code's shape. Match that; do not add restating comments.
- **Commit subjects are imperative with the reason attached** — "Share one device backend
  factory, since the reason for copying it was never true", "Build CI with `--locked` so a stale
  `Cargo.lock` fails instead of being rewritten".
- **`// ponytail:` marks a deliberate simplification** with its ceiling and upgrade path.
- **Protocol facts must cite their source** in the format from `docs/protocol/README.md`:
  `[src: inkscape-silhouette silhouette/Graphtec.py L120-155 (GPL-2.0+)]`, `[cap: ...pcapng
  #142-158]`, `[doc: <title>, <url>, section]`. This is GPL attribution compliance, not style.
  Raw captures are git-ignored; commit trimmed `.hex` fixtures with decoder tests instead.
- **Issues**: exactly one type label plus at most one provenance label — full rules in
  `docs/issue-labels.md`. Cite evidence as `path:line` and re-check the lines before filing.
- **Hardware-verified behaviour is tracked in `apps/desktop/MANUAL-CHECKLIST.md`**, with the
  device and date it was verified on. Anything that can only be confirmed on real hardware goes
  there rather than being claimed as tested.
- Design docs and plans live in `docs/superpowers/specs/` and `docs/superpowers/plans/`; each
  sub-project (protocol spike → drivers+CLI → editor shell → cut workflow → trace → print&cut)
  gets a spec then a plan then implementation.
