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
UPDATE_IPC_INVENTORY=1 cargo test -p desktop --test ipc_inventory   # regenerate ipc-inventory.json

P=apps/desktop/ui                        # every line below runs from the repo root
npm --prefix $P run dev                  # vite dev server on :5173
npm --prefix $P run build                # tsc + vite build -> dist/ (must be committed)
npm --prefix $P test                     # vitest run
npm --prefix $P test -- viewmodel        # one file
npm --prefix $P exec -- playwright install --with-deps chromium   # once per checkout; see below
npm --prefix $P run e2e                  # playwright; boots `npm run dev` itself

(cd apps/desktop && cargo tauri dev)     # cargo-tauri installed globally
(cd apps/desktop && cargo tauri build)   # -> target/release/bundle/{macos,dmg}

(cd tools && python3 -m pytest)          # spike tool tests
```

Every command above runs from the repository root, and the `cd`s that need to happen are inside
subshells for a reason: `cargo tauri` finds its config by searching **subfolders**, never parents,
so running it from `apps/desktop/ui` fails with "Couldn't recognize the current folder as a Tauri
project" rather than walking up to `apps/desktop`.

Two details in that block that look like noise and are not. `--with-deps` matches what CI installs:
without it Playwright downloads the browser but not the system libraries it needs, so `npm run e2e`
fails at launch on a fresh Linux box (a no-op on macOS, so keep it either way). And `exec --` is
required because `npm exec` otherwise eats `--with-deps` as its own config flag and silently drops
it — you get `Unknown cli config "--with-deps"` and a browser installed without dependencies.

`tauri.conf.json`'s `beforeDevCommand`/`beforeBuildCommand` are a bare `npm run build`, and that
is correct: Tauri runs them from the **frontend** directory (`apps/desktop/ui`), not from the
directory holding the config. Running `npm run build` yourself from `apps/desktop` does fail, which
makes the hook look broken — it is not. Do not "fix" it with `--prefix ui`; that resolves to
`ui/ui` and takes the packaged build down.

Three CI gates that fail on ordinary-looking commits:

- **`--locked`**: `Cargo.lock` is committed and CI refuses to rewrite it. Add a dependency →
  commit the lock in the same change.
- **`ui/dist` staleness**: `dist/` is committed because `tauri::generate_context!` reads
  `frontendDist` at Rust compile time, so the Rust CI job (no Node) needs it present. CI rebuilds
  and fails if the committed bundle differs. Touch `ui/src` → run
  `npm --prefix apps/desktop/ui run build` and commit `dist/`.
- **`apps/desktop/ipc-inventory.json` staleness**: the committed inventory of registered Tauri
  commands and their JavaScript argument names is what the e2e fake refuses calls outside of, so a
  stale copy is a call the frontend can still make and the real backend still rejects (#85). Change
  a command's name, its arguments, or its `#[tauri::command]` attribute → regenerate as above and
  commit the result. The test says so too, and names what moved.

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
can refuse a cut lives behind it: the stale-plan check (`doc_revision`), key→pass matching, and
`preflight` (finite/in-bounds/non-degenerate geometry, settings within machine range, document
meant for the connected machine, output size). A caller hands over `DocumentPasses` plus a
`PlanOptions` and gets a `CutPlan` or a `CutError`. Do not add a way around it; extend it.

`cuthulhu cut --group-by single` (the default) reaches it by asking the planner for one pass:
`plan_passes_with(&doc, Grouping::Single)` groups every cut shape into a single `DocumentPass`, in
document order, keyed `PassKey::All` — one pass by request, which is a different fact from the
pass of shapes with no visible paint (`no-color`). Since #148 every mode takes that same route;
the plain path has no planning function of its own.

A pass is identified by its `PassKey`, in one canonical string (`all`, `color:ff0000ff`,
`no-color`, `preset:cameo5-htv`, `no-preset`) that the CLI, the IPC payloads and the cut dialog's
row keys all share. Absence is its own token because a preset id is an unrestricted operator
string. **The `Grouping` travels in the `plan_cut`, `travel_for_order` and `cut` payloads**, and
the dialog holds it with the rows it produced: those are three separate round trips, and rows
keyed under one mode sent under another match passes holding different shapes.

A pass is not "enabled/disabled": a pass nobody lists in `PlanOptions::passes` is not cut.

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
  That fake is also where a mis-wired call is caught: it refuses any command name or payload key
  absent from `apps/desktop/ipc-inventory.json`, which `apps/desktop/tests/ipc_inventory.rs`
  generates from `main.rs`'s own `generate_handler!` registry. Names and argument names are checked
  in the language each is written in, which is why neither side parses the other.

### Document and files

- Every edit is a `Delta` whose inverse is derivable; `Editor` keeps inverse-delta undo and
  forward-delta redo stacks, so undo is just applying the inverse. Multi-step gestures commit as
  one `Delta`.
- `cutplan::doc_revision(doc)` is what "stale plan" compares against — a cut planned against a
  document that has since changed is refused, not cut.
- Project file is a zip: `manifest.json` (the source of truth — a `{ version, document }`
  envelope, currently version 2) plus `design.svg` (a best-effort interchange copy; unsupported
  node kinds become comments). Load probes `version` before deserializing the document, migrates
  through `fileio`'s ordered step table, and refuses a project from a newer build by name on both
  open and save-over. `Document::snapshot_json()` stays the bare, unversioned IPC shape. Saves are
  atomic — temp file in the destination directory, then rename.
- Material presets: builtins ship in `cutplan::presets`; user presets live in
  `<config_dir>/cuthulhu/presets.json` and the on-disk contract is *user entries only*
  (`builtin: false` forced on write). A preset is keyed on `(machine_id, id)` everywhere —
  shadowing, saving, deleting — because an operator's id is their own string (#153).
  The cut dialog manages them (`ui/src/cut/PresetEditor.tsx`, decisions in `presetDraft.ts`): every
  write goes through `save_preset`/`delete_preset`, which refuse a builtin's pair, an id-less or
  blank-named entry, a setting outside range, and a delete that removed nothing — the editor is a
  caller, never the enforcement. An id is minted from the name at creation and never moves again,
  because a `PresetAssignment` and a `preset:<id>` PassKey name a preset by it.
- Setting ranges live once, in `cutplan::preflight::SETTINGS_RANGES`, and reach the UI over
  `settings_ranges` — the arrangement `trace_controls` uses. Do not restate a bound in TypeScript.

## Conventions

- **`CONTEXT.md` is normative vocabulary.** It defines Document, Node, Delta, DocumentPass,
  PassKey, Grouping, PresetAssignment, DocumentPasses, PassSelection, CutPlan, Preflight,
  Settings, MaterialPreset, MachineProfile, MachineCaps, Driver, Transport, Job, Pass — each with
  an explicit `_Avoid_` list (no "layer" for DocumentPass, no "backend"/"plugin" for Driver, no
  "validation" for Preflight). `ColorPass` is retired with #148: a pass is named by a PassKey,
  which is a colour only under a colour Grouping. Use these words in code, comments, commits,
  and issues.
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

## Agent skills

### Issue tracker

Issues live in GitHub Issues on `vcolombo/cuthulhu`, via the `gh` CLI. See
`docs/agents/issue-tracker.md`.

### Triage labels

Default triage vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`,
`wontfix`), a workflow axis alongside the type/provenance labels. See
`docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` at the repo root plus `docs/adr/`. See `docs/agents/domain.md`.

### PR review

Three stages, in order, and each one coming back clean is the trigger for the next rather than the
finish line: **Copilot** on the PR, then **`/pr-review-toolkit:review-pr`** — the local agents
(code-reviewer, pr-test-analyzer, silent-failure-hunter, type-design-analyzer, comment-analyzer)
run against the diff — then **Codex, adversarially**, via `/codex:rescue` or
`codex exec review --commit <sha>` directly, since Codex is not a PR bot on this repo. Three
reviewers, three different blind spots, and this repo's history has each catching what the others
missed.

Brief Codex to attack the change rather than to look it over. The P1 worth having on PR #289 came
from asking it to enumerate every combination of "reached the host", "the answer settled" and
"first attempt" and say which arm each landed in; "does this look right" had already returned an
approval. Stages two and three comment nowhere on their own, so each gets its findings and their
dispositions posted to the PR — a gate triaged only in a session is invisible to whoever reads the
PR next.

Greptile and CodeRabbit are gone, and nothing waits on either. Their findings are still cited
across comments and tests by PR number — that is provenance and stays — but a wait on an
uninstalled bot is how PR #287 sat blocked on `Greptile Review`, a required check that nothing left
in the repo could produce; it has been dropped from `main`'s protection, which now requires `rust`,
`python-tools`, `ui` and `cross-pi`.
