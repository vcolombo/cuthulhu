<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Plain cut path + CutStatus Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the one cut path that reaches hardware without preflight, then narrow `DeviceManager`'s interface to a single `CutStatus` value so the device state machine stops being re-derived in three places across two languages.

**Architecture:** Phase 1 routes plain `cuthulhu cut` through `cutplan::plan_cut` by giving every imported path a uniform stroke (so all geometry is cuttable and exactly one `ColorPass` results), then transmits via `DeviceManager` instead of opening its own `Transport`. Phase 2 adds `driver_core::CutStatus` — phase, legal actions, pass position, byte progress — published by the worker to shared memory, returned by a non-blocking `status()`, and carried on every event; `DeviceState` becomes private so nothing outside `driver-core` can re-derive the machine.

**Tech Stack:** Rust 2021 (workspace crates `cli`, `cutplan`, `driver-core`, `driver-registry`, `document`, `fileio`), clap 4, serde, Tauri 2, TypeScript + React + Vitest, Playwright.

Architecture review: candidates 3 and 1 of the 2026-07-26 review. Related open issue: [#68](https://github.com/vcolombo/cuthulhu/issues/68) (whether cuttability follows the path or the stroke) — this plan deliberately does **not** decide it.

## Global Constraints

- Every new Rust/TS file starts with `// SPDX-License-Identifier: GPL-3.0-or-later` (first line). Markdown uses `<!-- SPDX-License-Identifier: GPL-3.0-or-later -->`.
- No AI attribution in commits or code.
- TDD is mandatory: write the failing test, run it, watch it fail **for the right reason**, then implement. Production code written before its test is deleted and redone.
- No test-only surface in production code. Dependencies are accepted as parameters, not constructed inside the function under test.
- `cargo test --workspace --locked` must pass. Adding a dependency means committing `Cargo.lock` in the same commit.
- Touching `apps/desktop/ui/src` means running `npm --prefix apps/desktop/ui run build` and committing `apps/desktop/ui/dist` in the same commit — CI fails on a stale bundle.
- `apps/desktop/ui/e2e/smoke.spec.ts` must stay green. Extend its mock; never weaken its assertions.
- `plan_passes` is **not** modified by this plan. The stroke rule stays exactly as it is; issue #68 remains open.
- Domain vocabulary comes from `CONTEXT.md`. New terms are added there in the same commit that introduces them, respecting existing `_Avoid_` lists (notably **Pass**: avoid run, cycle, layer).
- Machine ids are `cameo5` and `puma`, owned by `crates/driver-registry/src/lib.rs:15-16`. Do not add new string literals for them.

## Existing interfaces consumed (verified against the code)

```rust
// cutplan (crates/cutplan/src/{passes.rs,plan.rs,preflight.rs})
pub fn plan_passes(doc: &Document) -> Result<DocumentPasses, PlanError>
pub struct DocumentPasses { pub passes: Vec<ColorPass>, pub doc_revision: u64 }
pub struct ColorPass { pub color: Option<u32>, /* + flattened outlines */ }
pub struct PassSelection { pub color: Option<u32>, pub settings: Settings }
pub struct PlanOptions { pub passes: Vec<PassSelection>, pub expect_revision: Option<u64>, pub allow_out_of_bounds: bool }
pub fn plan_cut(planned: &DocumentPasses, profile: &MachineProfile, caps: &MachineCaps, opts: &PlanOptions)
    -> Result<CutPlan, CutError>
pub struct CutPlan { pub passes: Vec<PlannedPass> }
impl CutPlan { pub fn cut_passes(&self) -> Vec<CutPass> }
pub enum CutError { StalePlan { expected: u64, actual: u64 }, UnknownPassColor(Option<u32>), Preflight(PreflightError) }

// document (crates/document/src/{node.rs,delta.rs,commands.rs})
pub struct Delta(pub Vec<NodeOp>);
pub enum NodeOp { Add { parent: NodeId, node: Node, index: usize }, /* ... */ }
pub struct Node { pub id: NodeId, pub kind: NodeKind, pub transform: Affine, pub style: Style, pub children: Vec<NodeId> }
pub struct Style { pub stroke: Option<u32>, pub fill: Option<u32> }  // 0xRRGGBBAA
impl Document { pub fn new() -> Document; pub fn apply(&mut self, d: Delta) -> Delta /* returns inverse */ }

// fileio (crates/fileio/src/import.rs)
pub fn import_svg(bytes: &[u8], ids: &mut IdGen, parent: NodeId) -> Result<(Delta, Vec<String>), IoError>

// driver-core (crates/driver-core/src/lib.rs, manager.rs)
pub trait DeviceBackendFactory: Send + Sync {
    fn list_devices(&self) -> Vec<DeviceInfo>;
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>>;
    fn open_transport(&self, info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError>;
}
pub struct DeviceInfo { pub instance_id: String, pub machine_id: String, pub transport: TransportKind, pub candidate: bool }
pub enum TransportKind { Usb { locator: String }, Serial { path: String, baud: u32 } }
pub struct MachineCaps { pub supports_speed: bool, pub supports_force: bool, pub needs_operator_pass_confirm: bool }
#[derive(Default)] pub struct MockTransport {
    pub written: Vec<u8>,
    pub reads: VecDeque<Result<Vec<u8>, TransportError>>,
    pub write_results: VecDeque<Result<usize, TransportError>>,
}
impl DeviceManager {
    pub fn spawn(factory: Arc<dyn DeviceBackendFactory>) -> (DeviceManager, mpsc::Receiver<DeviceEvent>);
    pub fn connect(&self, info: DeviceInfo) -> Result<(), DeviceError>;
    pub fn cut(&self, passes: Vec<CutPass>) -> Result<u64, DeviceError>;
    pub fn snapshot(&self) -> DeviceState;      // Phase 2 replaces this with status()
    pub fn cancel(&self);
    pub fn resume(&self) -> Result<(), DeviceError>;
    pub fn confirm_pass_done(&self) -> Result<(), DeviceError>;
    pub fn shutdown(self);
}
pub struct CutPass { pub job: Job }

// cli (crates/cli/src/pipeline.rs) — existing helpers this plan reuses or removes
pub enum Device { Cameo5, Puma }
impl Device { pub fn from_id(s: &str) -> Result<Device, String>; pub fn driver(&self) -> Box<dyn Driver> }
pub fn doc_from_svg(svg: &[u8]) -> Result<Document, String>            // keep: --by-color uses it
pub fn pass_order(planned: &[ColorPass], skip: &[String], order: Option<String>) -> Result<Vec<Option<u32>>, String>
pub fn plan_cut_from_svg(...) -> Result<CutPlan, String>               // keep: --by-color
pub fn pass_stream_bytes(d: &dyn Driver, job: &Job, i: usize, total: usize) -> Result<Vec<u8>, String>  // keep
pub fn build_bytes(svg: &[u8], device: Device, settings: &Settings) -> Result<Vec<u8>, String>  // DELETED in Task 6
pub fn check_out_of_bounds_scope(allow: bool, by_color: bool) -> Result<(), String>             // DELETED in Task 3
pub fn check_interactive(is_tty: bool, pass_count: usize) -> Result<(), String>                 // keep
fn describe_cut_error(e: CutError) -> String                          // becomes pub(crate)-visible to new callers
```

## File Structure

**Phase 1**
- `crates/cli/src/pipeline.rs` — add `CUT_STROKE`, `doc_from_svg_all_cuttable`, `plan_plain_cut`, `check_color_flag_scope`, `Operator`; delete `build_bytes` and `check_out_of_bounds_scope`.
- `crates/cli/src/cut.rs` — **new.** The cut-driving loop, taking a `DeviceBackendFactory` as a parameter. Extracted from `main.rs` so both paths share it and both are testable.
- `crates/cli/src/lib.rs` — declare `pub mod cut;`.
- `crates/cli/src/main.rs` — argument handling only; delegate to `cut::run` for both paths. Delete the `UsbTransport`/`SerialTransport` match.
- `crates/cli/tests/plain_cut.rs` — **new.** Plain-path tests over `MockTransport`.
- `crates/cli/tests/dry_run.rs` — delete the `build_bytes` test; keep the `pass_stream_bytes` test.

**Phase 2**
- `crates/driver-core/src/status.rs` — **new.** `CutStatus`, `Phase`, `Actions`, `PassPosition`, `ByteProgress`, and the internal mapping from `DeviceState`.
- `crates/driver-core/src/manager.rs` — publish `CutStatus`; `status()`; `DeviceState` becomes `pub(crate)`; tests reworked.
- `crates/driver-core/src/lib.rs` — `pub mod status; pub use status::*;`.
- `apps/desktop/src/device.rs` — delete the state cache, the `Transmitting` synthesis and `is_active`; return `CutStatus`.
- `apps/desktop/src/main.rs` — bridge forwards `CutStatus`; close guard uses `CutStatus::is_active`.
- `apps/desktop/src/ipc.rs` — `get_device_state` returns `CutStatus`.
- `apps/desktop/ui/src/ipc.ts` — replace the `DeviceState` union with `CutStatus`.
- `apps/desktop/ui/src/cut/viewmodel.ts` — delete `dialogPhase`, `DevicePhase`, `canStartCut`, `dialogButtons`, `acceptEvent`, `terminalTransition`.
- `apps/desktop/ui/src/cut/viewmodel.test.ts` — delete the five corresponding `describe` blocks; keep `reorderPass` and `toCutRequest`.
- `apps/desktop/ui/src/cut/CutDialog.tsx`, `apps/desktop/ui/src/App.tsx` — render from `CutStatus`.
- `apps/desktop/ui/e2e/smoke.spec.ts` — mock emits `CutStatus`.
- `crates/cli/src/cut.rs` — loop switches on `CutStatus`.
- `CONTEXT.md` — add the **CutStatus** entry.
- `apps/desktop/MANUAL-CHECKLIST.md` — add the verification items.

---

# Phase 1 — Close the un-preflighted cut path

### Task 1: Make every imported path cuttable for a plain cut

**Files:**
- Modify: `crates/cli/src/pipeline.rs` (add after `doc_from_svg`, around line 66)
- Test: `crates/cli/src/pipeline.rs` (`#[cfg(test)] mod tests` at the end of the file)

**Interfaces:**
- Consumes: `fileio::import_svg`, `document::{Document, Delta, NodeOp}`, `cutplan::plan_passes`.
- Produces: `pub const CUT_STROKE: u32 = 0x000000FF;` and `pub fn doc_from_svg_all_cuttable(svg: &[u8]) -> Result<document::Document, String>`.

- [ ] **Step 1: Write the failing test**

In `crates/cli/src/pipeline.rs`, inside the test module:

```rust
    /// A fill-only SVG is what Illustrator, Inkscape and most clipart emit. The plain
    /// cut path has always cut it, so routing that path through `plan_passes` — which
    /// skips strokeless shapes — must not change what it cuts.
    #[test]
    fn fill_only_svg_plans_exactly_one_pass() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/><rect x="6" width="5" height="5" fill="#00ff00"/></svg>"##;
        let doc = doc_from_svg_all_cuttable(svg).expect("import");
        let planned = cutplan::plan_passes(&doc).expect("plan");
        assert_eq!(planned.passes.len(), 1, "all geometry belongs to one pass");
        assert_eq!(planned.passes[0].color, Some(CUT_STROKE));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cli fill_only_svg_plans_exactly_one_pass`
Expected: FAIL — `cannot find function 'doc_from_svg_all_cuttable' in this scope`.

- [ ] **Step 3: Write minimal implementation**

```rust
/// The stroke a plain cut gives every imported path. Opaque black, matching
/// `document::Style::default()`.
pub const CUT_STROKE: u32 = 0x000000FF;

/// Import `svg` for a plain (non-`--by-color`) cut: every path gets the same
/// stroke, so `plan_passes` finds all of it and groups it into exactly one
/// `ColorPass`.
///
/// This is what the plain path has always meant — cut everything in the file,
/// in one pass — stated explicitly so the cut can go through `plan_cut` and be
/// preflighted. It deliberately does not touch `plan_passes`' stroke rule; see
/// issue #68 for whether that rule should change at all.
pub fn doc_from_svg_all_cuttable(svg: &[u8]) -> Result<document::Document, String> {
    let mut doc = document::Document::new();
    let (mut delta, _skipped) = fileio::import_svg(svg, &mut doc.ids, doc.root)
        .map_err(|e| format!("SVG parse: {e:?}"))?;
    for op in delta.0.iter_mut() {
        if let document::NodeOp::Add { node, .. } = op {
            node.style.stroke = Some(CUT_STROKE);
        }
    }
    doc.apply(delta);
    Ok(doc)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cli fill_only_svg_plans_exactly_one_pass`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/pipeline.rs
git commit -m "Make a plain cut say out loud that everything in the file is cuttable"
```

---

### Task 2: Plan the plain cut through plan_cut

**Files:**
- Modify: `crates/cli/src/pipeline.rs`
- Test: `crates/cli/src/pipeline.rs` test module

**Interfaces:**
- Consumes: `CUT_STROKE`, `doc_from_svg_all_cuttable` (Task 1); `cutplan::{plan_passes, plan_cut, PlanOptions, PassSelection}`; `describe_cut_error`.
- Produces: `pub fn plan_plain_cut(svg: &[u8], device: Device, settings: &Settings, allow_out_of_bounds: bool) -> Result<cutplan::CutPlan, String>`.

- [ ] **Step 1: Write the failing tests**

Three behaviours: one pass, preflight actually refuses, and an empty file reports the right thing.

```rust
    #[test]
    fn plain_cut_plans_one_pass() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm">
            <rect width="5" height="5" fill="#ff0000"/></svg>"##;
        let plan = plan_plain_cut(svg, Device::Cameo5, &Settings::default(), false).expect("plan");
        assert_eq!(plan.passes.len(), 1);
    }

    /// The whole point of the change: the plain path is preflighted. A shape past the
    /// bed's edge was silently sent to the machine before.
    #[test]
    fn plain_cut_refuses_out_of_bounds_geometry() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000mm" height="10mm">
            <rect x="9000" width="500" height="5" fill="#000000"/></svg>"##;
        let err = plan_plain_cut(svg, Device::Cameo5, &Settings::default(), false)
            .expect_err("out of bounds must be refused");
        assert!(err.contains("outside"), "unexpected message: {err}");
        // ...and the escape hatch works, now that there is a check to overrule.
        assert!(plan_plain_cut(svg, Device::Cameo5, &Settings::default(), true).is_ok());
    }

    /// With no paths at all, `plan_passes` yields no passes, so the requested colour
    /// matches nothing. Without the empty check that surfaces as `UnknownPassColor`,
    /// which reads as an internal error rather than "there is nothing here".
    #[test]
    fn plain_cut_of_an_empty_svg_says_nothing_to_cut() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"></svg>"##;
        let err = plan_plain_cut(svg, Device::Cameo5, &Settings::default(), false).expect_err("empty");
        assert_eq!(err, "no cuttable paths in SVG");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cli plain_cut`
Expected: FAIL — `cannot find function 'plan_plain_cut' in this scope`.

- [ ] **Step 3: Write minimal implementation**

```rust
/// Plan a plain cut: all geometry, one pass, validated through `plan_cut` — the
/// same entry point the desktop and `--by-color` use.
pub fn plan_plain_cut(
    svg: &[u8],
    device: Device,
    settings: &Settings,
    allow_out_of_bounds: bool,
) -> Result<cutplan::CutPlan, String> {
    let doc = doc_from_svg_all_cuttable(svg)?;
    let planned = cutplan::plan_passes(&doc).map_err(|e| format!("plan: {e:?}"))?;
    // Checked here rather than left to `plan_cut`: with no passes at all, asking for
    // CUT_STROKE is an unmatched colour, and "no pass matches color" describes the
    // request instead of the file.
    if planned.passes.is_empty() {
        return Err("no cuttable paths in SVG".into());
    }
    let passes = vec![cutplan::PassSelection { color: Some(CUT_STROKE), settings: settings.clone() }];
    let driver = device.driver();
    let opts = cutplan::PlanOptions { passes, expect_revision: None, allow_out_of_bounds };
    cutplan::plan_cut(&planned, driver.profile(), &driver.caps(), &opts).map_err(describe_cut_error)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cli plain_cut`
Expected: PASS (3 tests)

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/pipeline.rs
git commit -m "Preflight the plain cut path, which reached the machine unchecked"
```

---

### Task 3: Fix the flag scopes the change alters

**Files:**
- Modify: `crates/cli/src/pipeline.rs:152-158` (delete `check_out_of_bounds_scope`), add `check_color_flag_scope`
- Test: `crates/cli/src/pipeline.rs` test module (delete the `check_out_of_bounds_scope` tests if present)

**Interfaces:**
- Produces: `pub fn check_color_flag_scope(skip_colors: &[String], order: &Option<String>, by_color: bool) -> Result<(), String>`.
- Removes: `check_out_of_bounds_scope` — every caller must be updated in this task.

- [ ] **Step 1: Write the failing test**

```rust
    /// `--skip-color` and `--order` name colours, and a plain cut deliberately
    /// collapses every colour into one pass. Accepting them silently — which is what
    /// happened before — reports success for a flag that did nothing.
    #[test]
    fn colour_flags_are_refused_without_by_color() {
        let red = vec!["FF0000FF".to_string()];
        let err = check_color_flag_scope(&red, &None, false).expect_err("must refuse");
        assert!(err.contains("--skip-color"), "unexpected message: {err}");
        let err = check_color_flag_scope(&[], &Some("FF0000FF".into()), false).expect_err("must refuse");
        assert!(err.contains("--order"), "unexpected message: {err}");
        // Both are fine with --by-color, and absence is fine either way.
        assert!(check_color_flag_scope(&red, &Some("FF0000FF".into()), true).is_ok());
        assert!(check_color_flag_scope(&[], &None, false).is_ok());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cli colour_flags_are_refused_without_by_color`
Expected: FAIL — `cannot find function 'check_color_flag_scope' in this scope`.

- [ ] **Step 3: Write minimal implementation**

Delete `check_out_of_bounds_scope` (`pipeline.rs:152-158`) entirely — `--allow-out-of-bounds` now relaxes a check that runs on both paths — and add:

```rust
/// `--skip-color` and `--order` select and sequence colours, which only a
/// `--by-color` cut has. A plain cut puts every colour in one pass, so these
/// flags cannot do anything there and are refused rather than ignored.
pub fn check_color_flag_scope(
    skip_colors: &[String],
    order: &Option<String>,
    by_color: bool,
) -> Result<(), String> {
    if by_color {
        return Ok(());
    }
    if !skip_colors.is_empty() {
        return Err("--skip-color applies to --by-color cuts; a plain cut is one pass over every colour".into());
    }
    if order.is_some() {
        return Err("--order applies to --by-color cuts; a plain cut is one pass over every colour".into());
    }
    Ok(())
}
```

- [ ] **Step 4: Run the crate's tests**

Run: `cargo test -p cli`
Expected: the new test PASSES. Compilation fails at `crates/cli/src/main.rs:128` where `check_out_of_bounds_scope` was called — that call is replaced in Task 7. To keep this task independently testable, update that one line now to `check_color_flag_scope(&skip_color, &order, by_color)?;`.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/pipeline.rs crates/cli/src/main.rs
git commit -m "Refuse the colour flags a plain cut cannot honour, and drop the guard preflight made obsolete"
```

---

### Task 4: Extract the cut loop so it can be driven by a test

**Files:**
- Create: `crates/cli/src/cut.rs`
- Modify: `crates/cli/src/lib.rs` (add `pub mod cut;`)
- Test: `crates/cli/tests/plain_cut.rs` (created in Task 5)

**Interfaces:**
- Consumes: `driver_core::{DeviceBackendFactory, DeviceInfo, manager::{DeviceManager, DeviceState}}`, `cutplan::CutPlan`, `pipeline::check_interactive`.
- Produces:
  ```rust
  pub enum Operator { Interactive, Unattended }
  pub fn run(plan: &cutplan::CutPlan, info: DeviceInfo,
             factory: std::sync::Arc<dyn DeviceBackendFactory>,
             operator: Operator) -> Result<(), String>
  ```

- [ ] **Step 1: Write the file**

There is no test in this step — Task 5 supplies it, because the test needs a fake factory that is worth its own task. This task is pure extraction: the loop below is `main.rs:236-276` with `mgr.snapshot()` unchanged and the two waits routed through `Operator`.

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
//! Driving a planned cut on a real device: connect, submit, and answer the
//! machine's pauses until the job ends.
//!
//! Takes its `DeviceBackendFactory` as a parameter rather than building one, so
//! the same code runs against hardware and against `MockTransport`.
use std::sync::Arc;

use driver_core::manager::{DeviceManager, DeviceState};
use driver_core::{DeviceBackendFactory, DeviceInfo};

/// Who answers the machine's pauses.
///
/// `Unattended` exists because a pass on a machine that cannot be polled
/// (`MachineCaps::needs_operator_pass_confirm`) otherwise blocks on stdin, and a
/// plain cut is often scripted. It acknowledges as soon as the bytes are sent,
/// which is exactly what the plain path did before it went through
/// `DeviceManager` — the host queue draining is not the machine finishing, so it
/// says so on stderr rather than pretending the cut is verified.
pub enum Operator {
    Interactive,
    Unattended,
}

impl Operator {
    /// Wait for acknowledgement. `false` means a cancel landed while waiting.
    fn wait_ack(&self, prompt: &str, mgr: &DeviceManager) -> bool {
        match self {
            Operator::Interactive => {
                println!("{prompt}");
                wait_for_enter_or_cancel(mgr)
            }
            Operator::Unattended => {
                eprintln!("{prompt}: assuming done (stdin is not a terminal; completion not verified)");
                true
            }
        }
    }
}

/// Connect, cut, and drive the job to its end. `Ok(())` covers a completed cut
/// and a cancelled one; a device fault is an `Err`.
pub fn run(
    plan: &cutplan::CutPlan,
    info: DeviceInfo,
    factory: Arc<dyn DeviceBackendFactory>,
    operator: Operator,
) -> Result<(), String> {
    let total = plan.passes.len();
    let (mgr, _events) = DeviceManager::spawn(factory);
    let mgr = Arc::new(mgr);
    mgr.connect(info).map_err(|e| format!("connect: {e:?}"))?;

    // ponytail: the handler holds a permanent Arc clone for the life of the
    // process, so `mgr` is never uniquely owned again — skip a graceful
    // `shutdown()` and let the (short-lived CLI) process exit reap the worker.
    let ctrlc_mgr = mgr.clone();
    ctrlc::set_handler(move || ctrlc_mgr.cancel()).map_err(|e| format!("ctrlc: {e}"))?;

    mgr.cut(plan.cut_passes()).map_err(|e| format!("cut: {e:?}"))?;

    loop {
        match mgr.snapshot() {
            DeviceState::WaitingForColorSwap { next_pass_index, .. } => {
                // The colour is why the operator is being interrupted — it says which
                // one to load — so it comes from the plan being cut, not a side list.
                let prompt = format!(
                    "Pass {}/{} (color {}): swap tool, press Enter to resume",
                    next_pass_index + 1, total, pass_color(plan, next_pass_index),
                );
                if !operator.wait_ack(&prompt, &mgr) {
                    continue; // re-check snapshot: cancel() already landed
                }
                mgr.resume().map_err(|e| format!("resume: {e:?}"))?;
            }
            DeviceState::AwaitingCompletion { pass_index, .. } => {
                let prompt = format!(
                    "Pass {}/{} (color {}) cutting; press Enter once the machine finishes",
                    pass_index + 1, total, pass_color(plan, pass_index),
                );
                if !operator.wait_ack(&prompt, &mgr) {
                    continue;
                }
                mgr.confirm_pass_done().map_err(|e| format!("confirm: {e:?}"))?;
            }
            DeviceState::Idle => {
                println!("done: {total} passes cut");
                return Ok(());
            }
            DeviceState::Cancelled { pass_index, submitted_bytes, .. } => {
                println!("cancelled at pass {pass_index} ({submitted_bytes} bytes sent)");
                return Ok(());
            }
            DeviceState::Error(e) => return Err(format!("device error: {e:?}")),
            _ => return Err("unexpected device state".into()),
        }
    }
}

/// The colour of pass `i`, for a prompt. Out-of-range cannot happen for a plan
/// the manager is cutting, but a prompt is no place to panic if it did.
fn pass_color(plan: &cutplan::CutPlan, i: usize) -> String {
    plan.passes.get(i).map(|p| format_pass_color(p.color)).unwrap_or_else(|| "?".into())
}

/// Also moved here from `main.rs`, so the prompt wording lives beside the loop
/// that prints it. `main.rs` calls `cut::format_pass_color` until Task 7 removes
/// its copy of the loop.
pub fn format_pass_color(color: Option<u32>) -> String { /* body unchanged from main.rs */ }

/// Block until the operator presses Enter (`true`) or a cancel lands via
/// Ctrl-C/`DeviceManager::cancel` (`false`). The reader thread is left parked on
/// stdin if cancel wins — fine for a process that's about to exit.
fn wait_for_enter_or_cancel(mgr: &DeviceManager) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = std::io::stdin().read_line(&mut buf);
        let _ = tx.send(());
    });
    loop {
        if rx.try_recv().is_ok() {
            return true;
        }
        if matches!(mgr.snapshot(), DeviceState::Cancelled { .. }) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
```

- [ ] **Step 2: Declare the module**

In `crates/cli/src/lib.rs`, add `pub mod cut;` beside the existing `pub mod pipeline;`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo test -p cli`
Expected: compiles; existing tests pass. `main.rs` still has its own copy of the loop — Task 7 removes it.

- [ ] **Step 4: Commit**

```bash
git add crates/cli/src/cut.rs crates/cli/src/lib.rs
git commit -m "Give the cut loop a seam that accepts a device backend"
```

---

### Task 5: Test the plain path end to end over MockTransport

**Files:**
- Create: `crates/cli/tests/plain_cut.rs`

**Interfaces:**
- Consumes: `cli::pipeline::plan_plain_cut` (Task 2), `cli::cut::{run, Operator}` (Task 4), `driver_core::{Driver, MockTransport, DeviceBackendFactory}`.
- Produces: nothing consumed by later tasks.

The fake driver deliberately sets `needs_operator_pass_confirm: true`, so the job parks in `AwaitingCompletion` and `Operator::Unattended` drives it to completion without any transport reads — deterministic, no status-poll scripting needed.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
//! The plain (`cuthulhu cut`, no `--by-color`) path, driven against a fake
//! device: the bytes a real machine would receive, and the refusals that stop
//! bytes being produced at all.
use std::sync::{Arc, Mutex};

use cli::cut::{run, Operator};
use cli::pipeline::{plan_plain_cut, Device};
use driver_core::{
    DeviceBackendFactory, DeviceInfo, Driver, DriverError, Job, MachineCaps, MachineProfile,
    MockTransport, Settings, Transport, TransportError, TransportKind,
};

const SQUARE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm">
    <rect width="10" height="10" fill="#ff0000"/></svg>"##;

struct FakeDriver {
    profile: MachineProfile,
}
impl Driver for FakeDriver {
    fn profile(&self) -> &MachineProfile { &self.profile }
    fn caps(&self) -> MachineCaps {
        // Needs an operator to confirm, so the job parks instead of polling.
        MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true }
    }
    fn session_begin(&self) -> Vec<u8> { b"BEGIN".to_vec() }
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError> {
        Ok(format!("PASS{}", pass.polylines.len()).into_bytes())
    }
    fn pass_park(&self) -> Vec<u8> { b"PARK".to_vec() }
    fn session_end(&self) -> Vec<u8> { b"END".to_vec() }
    fn abort_bytes(&self) -> Option<Vec<u8>> { None }
}

/// Hands out one transport whose written bytes the test can inspect afterwards.
struct TestFactory {
    written: Arc<Mutex<Vec<u8>>>,
}
impl DeviceBackendFactory for TestFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> { vec![info()] }
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        Some(Box::new(FakeDriver {
            profile: MachineProfile {
                id: machine_id.to_string(),
                name: "fake".into(),
                width_mm: 330.0,
                height_mm: 3000.0,
            },
        }))
    }
    fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
        Ok(Box::new(RecordingTransport { inner: MockTransport::default(), sink: self.written.clone() }))
    }
}

struct RecordingTransport {
    inner: MockTransport,
    sink: Arc<Mutex<Vec<u8>>>,
}
impl Transport for RecordingTransport {
    fn write(&mut self, b: &[u8]) -> Result<usize, TransportError> {
        self.sink.lock().unwrap().extend_from_slice(b);
        self.inner.write(b)
    }
    fn read(&mut self, buf: &mut [u8], t: std::time::Duration) -> Result<usize, TransportError> {
        self.inner.read(buf, t)
    }
}

fn info() -> DeviceInfo {
    DeviceInfo {
        instance_id: "test:0".into(),
        machine_id: "cameo5".into(),
        transport: TransportKind::Usb { locator: "0:0".into() },
        candidate: false,
    }
}

#[test]
fn a_plain_cut_sends_one_framed_pass() {
    let plan = plan_plain_cut(SQUARE, Device::Cameo5, &Settings::default(), false).expect("plan");
    assert_eq!(plan.passes.len(), 1, "a plain cut is one pass");

    let written = Arc::new(Mutex::new(Vec::new()));
    let factory = Arc::new(TestFactory { written: written.clone() });
    run(&plan, info(), factory, Operator::Unattended).expect("cut");

    let bytes = String::from_utf8(written.lock().unwrap().clone()).expect("utf8");
    assert!(bytes.starts_with("BEGIN"), "session must open once: {bytes}");
    assert!(bytes.ends_with("END"), "session must close once: {bytes}");
    assert_eq!(bytes.matches("PASS").count(), 1, "exactly one pass: {bytes}");
    assert!(!bytes.contains("PARK"), "no inter-pass park on a single pass: {bytes}");
}

/// Preflight refusals must happen before a transport is ever opened.
#[test]
fn geometry_off_the_bed_never_reaches_a_transport() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000mm" height="10mm">
        <rect x="9000" width="500" height="5" fill="#000000"/></svg>"##;
    assert!(plan_plain_cut(svg, Device::Cameo5, &Settings::default(), false).is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cli --test plain_cut`
Expected: FAIL at compile time until Tasks 1, 2 and 4 are in; then both tests must pass. If `a_plain_cut_sends_one_framed_pass` fails on the `PASS` count, the plan produced more than one pass — Task 1's uniform stroke is not being applied.

- [ ] **Step 3: Make them pass**

No new production code should be required. If a test fails, fix the production code, not the assertion.

- [ ] **Step 4: Run the whole suite**

Run: `cargo test --workspace --locked`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/cli/tests/plain_cut.rs
git commit -m "Test the plain cut path against a fake device, which nothing could do before"
```

---

### Task 6: Delete build_bytes and route --dry-run through the plan

**Files:**
- Modify: `crates/cli/src/pipeline.rs` (delete `build_bytes`, lines 27-38)
- Modify: `crates/cli/tests/dry_run.rs` (delete the `build_bytes` test at lines 6-13)
- Modify: `crates/cli/src/main.rs` (dry-run branch)

**Interfaces:**
- Consumes: `plan_plain_cut` (Task 2), `pass_stream_bytes` (existing).
- Removes: `pipeline::build_bytes`.

- [ ] **Step 1: Write the failing test**

The test asserts behaviour that does **not** hold yet: today `--dry-run` goes through
`build_bytes`, which runs no preflight and cheerfully prints bytes for geometry off the
bed. Add to `crates/cli/tests/dry_run.rs`:

```rust
/// A dry run must refuse what a real cut would refuse. Through `build_bytes` it did
/// not: off-bed geometry printed bytes, so `--dry-run` reported a cut that
/// `--by-color` and the desktop would both have rejected.
#[test]
fn plain_dry_run_refuses_geometry_off_the_bed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let svg = dir.path().join("off-bed.svg");
    std::fs::write(&svg, br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000mm" height="10mm">
        <rect x="9000" width="500" height="5" fill="#000000"/></svg>"##).expect("write");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cuthulhu"))
        .args(["cut", svg.to_str().unwrap(), "--device", "cameo5", "--dry-run"])
        .output()
        .expect("run");

    assert!(!out.status.success(), "off-bed dry run must fail");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("outside"), "expected a bounds refusal, got: {err}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cli --test dry_run plain_dry_run_refuses_geometry_off_the_bed`
Expected: FAIL — the command succeeds and prints hex, because `build_bytes` never
preflights. That failure is the reason this task exists.

- [ ] **Step 3: Delete build_bytes and its test**

Delete `pipeline::build_bytes` (`pipeline.rs:27-38`) and the `build_bytes` test in `crates/cli/tests/dry_run.rs:6-13`. Update `main.rs`'s dry-run branch to use the plan:

```rust
            if !by_color {
                let plan = plan_plain_cut(&svg, device, &settings, allow_out_of_bounds)?;
                let driver = device.driver();
                let bytes = pass_stream_bytes(driver.as_ref(), &plan.passes[0].job, 0, 1)?;
                if dry_run {
                    print_hex_ascii(&bytes);
                    return Ok(());
                }
                // transmit path lands in Task 7
            }
```

- [ ] **Step 4: Run the suite**

Run: `cargo test --workspace --locked`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/pipeline.rs crates/cli/src/main.rs crates/cli/tests/dry_run.rs
git commit -m "Delete the byte builder that existed only to skip preflight"
```

---

### Task 7: Wire the plain path to DeviceManager and delete the second transport match

**Files:**
- Modify: `crates/cli/src/main.rs:126-158` (plain branch), `main.rs:220-276` (delete the extracted loop), imports at `main.rs:4-7`
- Test: `crates/cli/tests/plain_cut.rs` (already covers it)

**Interfaces:**
- Consumes: `cut::run`, `cut::Operator` (Task 4), `plan_plain_cut` (Task 2), `resolve_device_info` (existing, `main.rs:280`).
- Removes: the `UsbTransport::open()` / `SerialTransport::open()` match and the `driver_core::write_all` call from `main.rs`.

- [ ] **Step 1: Replace the plain branch**

```rust
        Command::Cut { file, device, dry_run, speed, force, port, baud, by_color, skip_color, order, allow_out_of_bounds } => {
            let device = Device::from_id(&device)?;
            check_color_flag_scope(&skip_color, &order, by_color)?;
            let svg = std::fs::read(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
            let settings = Settings { speed, force, repeat_count: 1 };

            if !by_color {
                let plan = plan_plain_cut(&svg, device, &settings, allow_out_of_bounds)?;
                if dry_run {
                    let driver = device.driver();
                    let bytes = pass_stream_bytes(driver.as_ref(), &plan.passes[0].job, 0, 1)?;
                    print_hex_ascii(&bytes);
                    return Ok(());
                }
                let info = resolve_device_info(device, port.as_deref(), baud)?;
                let factory: Arc<dyn DeviceBackendFactory> = Arc::new(HardwareBackendFactory);
                return cut::run(&plan, info, factory, operator());
            }

            cut_by_color(&svg, device, &settings, &skip_color, order, dry_run, port, baud, allow_out_of_bounds)
        }
```

Add the helper that decides who is answering, replacing the bare `check_interactive` call for the plain path:

```rust
/// A terminal means a human can answer the machine's pauses; anything else
/// (a script, a CI job) must not be left blocking on stdin.
fn operator() -> cut::Operator {
    if std::io::stdin().is_terminal() { cut::Operator::Interactive } else { cut::Operator::Unattended }
}
```

- [ ] **Step 2: Point cut_by_color at the shared loop**

In `cut_by_color`, delete the inline loop (`main.rs:236-276`) and `wait_for_enter_or_cancel` (`main.rs:302-317`), replacing the tail with:

```rust
    check_interactive(std::io::stdin().is_terminal(), passes.len())?;
    let info = resolve_device_info(device, port.as_deref(), baud)?;
    let factory: Arc<dyn DeviceBackendFactory> = Arc::new(HardwareBackendFactory);
    cut::run(&plan, info, factory, operator())
}
```

Remove the now-unused `Transport`, `TransportKind`, `DeviceState`, `DeviceManager` imports from `main.rs` and drop `driver_silhouette`/`driver_hpgl` from `crates/cli/Cargo.toml` **only if** nothing else in the crate uses them — check with `grep -rn "driver_silhouette\|driver_hpgl" crates/cli/src` first and leave them if `Device::driver()` still needs them.

- [ ] **Step 3: Run the suite**

Run: `cargo test --workspace --locked`
Expected: PASS. Then confirm no transport is opened outside the registry:

Run: `grep -rn "UsbTransport::open\|SerialTransport::open" crates/cli/src apps/desktop/src`
Expected: no matches (the only remaining callers are inside `crates/driver-registry/src/lib.rs`).

- [ ] **Step 4: Verify the dry-run output by hand**

Run: `cargo run -p cli -- cut /tmp/square.svg --device puma --dry-run`
Expected: hex/ASCII showing `IN;` … `PU;` around one pass. Compare against the pre-change output for the same file — the geometry bytes should be identical.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/main.rs crates/cli/Cargo.toml
git commit -m "Send every CLI cut through the planner, and open transports in one place"
```

---

### Task 8: Record Phase 1 in the manual checklist

**Files:**
- Modify: `apps/desktop/MANUAL-CHECKLIST.md`

- [ ] **Step 1: Add the items**

```markdown
## CLI plain cut path (architecture review candidate 3)

- [ ] `cuthulhu cut fill-only.svg --device cameo5 --dry-run` — a fill-only SVG still produces bytes (one pass).
- [ ] `cuthulhu cut off-bed.svg --device cameo5` — refused with the out-of-bounds message, nothing sent.
- [ ] `cuthulhu cut off-bed.svg --device cameo5 --allow-out-of-bounds` — sends.
- [ ] `cuthulhu cut a.svg --skip-color FF0000FF` — refused, naming `--by-color`.
- [ ] On hardware: a plain cut on the Cameo 5 completes, and Ctrl-C mid-cut stops it.
- [ ] Scripted (stdin redirected from /dev/null): `cuthulhu cut a.svg --device puma --port …` completes without blocking, and prints the completion-not-verified note.
```

- [ ] **Step 2: Commit**

```bash
git add apps/desktop/MANUAL-CHECKLIST.md
git commit -m "List what a human still has to check on the plain cut path"
```

---

# Phase 2 — Collapse the device state machine behind CutStatus

### Task 9: Add CutStatus and the mapping from the internal state

**Files:**
- Create: `crates/driver-core/src/status.rs`
- Modify: `crates/driver-core/src/lib.rs` (add `pub mod status; pub use status::*;`)
- Test: `crates/driver-core/src/status.rs` test module

**Interfaces:**
- Consumes: `crate::manager::{DeviceState, DeviceError}`.
- Produces:
  ```rust
  pub struct CutStatus { pub phase: Phase, pub actions: Actions,
                         pub pass: Option<PassPosition>, pub sent: Option<ByteProgress>,
                         pub error: Option<DeviceError> }
  pub enum Phase { Disconnected, Connecting, Disconnecting, Idle, Sending,
                   AwaitingConfirmation, AwaitingColorSwap, Cancelling, Done, Failed }
  pub struct Actions { pub cut: bool, pub cancel: bool, pub resume: bool, pub confirm: bool }
  pub struct PassPosition { pub index: usize, pub total: usize }
  pub struct ByteProgress { pub sent: usize, pub total: usize }
  impl CutStatus { pub fn is_active(&self) -> bool }
  pub(crate) fn status_of(state: &DeviceState, total_passes: usize) -> CutStatus
  ```

`Phase` carries the three connection phases as well as the seven cut phases: `apps/desktop/ui/src/cut/viewmodel.ts:223-241` maps `Disconnected` and `Connecting` today, and the dialog needs them to say "no device".

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{DeviceError, DeviceState};

    /// A caller must be able to render its buttons from `actions` alone, without
    /// knowing which phase permits which call. These four cases are the guards at
    /// manager.rs's `cut`, `resume` and `confirm_pass_done`.
    #[test]
    fn actions_state_which_calls_are_legal() {
        let idle = status_of(&DeviceState::Idle, 0);
        assert_eq!(idle.phase, Phase::Idle);
        assert!(idle.actions.cut && !idle.actions.cancel && !idle.actions.resume && !idle.actions.confirm);

        let swap = status_of(&DeviceState::WaitingForColorSwap { job_id: 1, next_pass_index: 1 }, 3);
        assert_eq!(swap.phase, Phase::AwaitingColorSwap);
        assert!(swap.actions.resume && swap.actions.cancel && !swap.actions.cut && !swap.actions.confirm);

        let await_done = status_of(&DeviceState::AwaitingCompletion { job_id: 1, pass_index: 0 }, 2);
        assert_eq!(await_done.phase, Phase::AwaitingConfirmation);
        assert!(await_done.actions.confirm && await_done.actions.cancel && !await_done.actions.resume);

        let sending = status_of(
            &DeviceState::Transmitting { job_id: 1, pass_index: 0, submitted_bytes: 40, total_bytes: 100 }, 2);
        assert_eq!(sending.phase, Phase::Sending);
        assert!(sending.actions.cancel && !sending.actions.cut);
    }

    /// Pass position and byte progress ride along with the phase, so a caller never
    /// correlates a progress event against a separate state read.
    #[test]
    fn sending_carries_pass_and_byte_position() {
        let s = status_of(
            &DeviceState::Transmitting { job_id: 7, pass_index: 1, submitted_bytes: 4096, total_bytes: 20480 }, 3);
        assert_eq!(s.pass, Some(PassPosition { index: 1, total: 3 }));
        assert_eq!(s.sent, Some(ByteProgress { sent: 4096, total: 20480 }));
    }

    /// A cut that ended is `Done` whether it finished or was cancelled; only a fault
    /// is `Failed`, and it carries the reason.
    #[test]
    fn terminal_phases_are_distinguishable() {
        assert_eq!(status_of(&DeviceState::Idle, 0).phase, Phase::Idle);
        let cancelled = status_of(
            &DeviceState::Cancelled { job_id: 1, pass_index: 0, submitted_bytes: 10, completion_known: false }, 1);
        assert_eq!(cancelled.phase, Phase::Done);
        let failed = status_of(&DeviceState::Error(DeviceError::Timeout), 1);
        assert_eq!(failed.phase, Phase::Failed);
        assert_eq!(failed.error, Some(DeviceError::Timeout));
    }

    /// Replaces `apps/desktop/src/device.rs`'s five-variant `is_active` match, which
    /// the window-close guard uses to decide whether to block a quit.
    #[test]
    fn is_active_covers_every_mid_flight_phase() {
        for state in [
            DeviceState::Transmitting { job_id: 1, pass_index: 0, submitted_bytes: 0, total_bytes: 1 },
            DeviceState::AwaitingCompletion { job_id: 1, pass_index: 0 },
            DeviceState::WaitingForColorSwap { job_id: 1, next_pass_index: 1 },
            DeviceState::CancelRequested { job_id: 1 },
            DeviceState::Stopping { job_id: 1 },
        ] {
            assert!(status_of(&state, 2).is_active(), "{state:?} is mid-flight");
        }
        for state in [DeviceState::Idle, DeviceState::Disconnected] {
            assert!(!status_of(&state, 0).is_active(), "{state:?} is not mid-flight");
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p driver-core status`
Expected: FAIL — `file not found for module 'status'`.

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
//! What a caller is told about a cut: where it has got to, and what may be done
//! next.
//!
//! This is the whole of `DeviceManager`'s reporting interface. The internal
//! state machine is not part of it — callers that branch on which phase permits
//! which call end up re-deriving the machine, which is what `actions` exists to
//! prevent.
use serde::Serialize;

use crate::manager::{DeviceError, DeviceState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Phase {
    Disconnected,
    Connecting,
    Disconnecting,
    Idle,
    Sending,
    /// The machine cannot be polled, so a human confirms the pass finished.
    AwaitingConfirmation,
    AwaitingColorSwap,
    Cancelling,
    Done,
    Failed,
}

/// Which calls are legal right now. A caller renders its controls from this and
/// never needs to know the phase-to-permission rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Actions {
    pub cut: bool,
    pub cancel: bool,
    pub resume: bool,
    pub confirm: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PassPosition {
    pub index: usize,
    pub total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ByteProgress {
    pub sent: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutStatus {
    pub phase: Phase,
    pub actions: Actions,
    pub pass: Option<PassPosition>,
    pub sent: Option<ByteProgress>,
    pub error: Option<DeviceError>,
}

impl CutStatus {
    /// True while a cut is mid-flight — what the window-close guard asks.
    pub fn is_active(&self) -> bool {
        matches!(
            self.phase,
            Phase::Sending | Phase::AwaitingConfirmation | Phase::AwaitingColorSwap | Phase::Cancelling
        )
    }
}

pub(crate) fn status_of(state: &DeviceState, total_passes: usize) -> CutStatus {
    let pass = |index: usize| Some(PassPosition { index, total: total_passes });
    let (phase, actions, pass, sent, error) = match state {
        DeviceState::Disconnected => (Phase::Disconnected, Actions::default(), None, None, None),
        DeviceState::Connecting => (Phase::Connecting, Actions::default(), None, None, None),
        DeviceState::Disconnecting => (Phase::Disconnecting, Actions::default(), None, None, None),
        DeviceState::Idle => (Phase::Idle, Actions { cut: true, ..Actions::default() }, None, None, None),
        DeviceState::Transmitting { pass_index, submitted_bytes, total_bytes, .. } => (
            Phase::Sending,
            Actions { cancel: true, ..Actions::default() },
            pass(*pass_index),
            Some(ByteProgress { sent: *submitted_bytes, total: *total_bytes }),
            None,
        ),
        DeviceState::AwaitingCompletion { pass_index, .. } => (
            Phase::AwaitingConfirmation,
            Actions { cancel: true, confirm: true, ..Actions::default() },
            pass(*pass_index),
            None,
            None,
        ),
        DeviceState::WaitingForColorSwap { next_pass_index, .. } => (
            Phase::AwaitingColorSwap,
            Actions { cancel: true, resume: true, ..Actions::default() },
            pass(*next_pass_index),
            None,
            None,
        ),
        DeviceState::CancelRequested { .. } | DeviceState::Stopping { .. } => {
            (Phase::Cancelling, Actions::default(), None, None, None)
        }
        // A cancelled job has ended. `cut` is legal again, exactly as
        // `manager.rs`'s cut guard already allows.
        DeviceState::Cancelled { pass_index, submitted_bytes, .. } => (
            Phase::Done,
            Actions { cut: true, ..Actions::default() },
            pass(*pass_index),
            Some(ByteProgress { sent: *submitted_bytes, total: *submitted_bytes }),
            None,
        ),
        DeviceState::Error(e) => (Phase::Failed, Actions::default(), None, None, Some(e.clone())),
    };
    CutStatus { phase, actions, pass, sent, error }
}
```

Add to `crates/driver-core/src/lib.rs`, beside `pub mod manager;`:

```rust
pub mod status;
pub use status::{Actions, ByteProgress, CutStatus, PassPosition, Phase};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p driver-core status`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add crates/driver-core/src/status.rs crates/driver-core/src/lib.rs
git commit -m "Say where a cut has got to and what may be done next, in one value"
```

---

### Task 10: Publish CutStatus so reading it never blocks

**Files:**
- Modify: `crates/driver-core/src/manager.rs` (`DeviceManager` struct ~line 80, `spawn` ~86, `worker_loop` ~470, the `emit` helper)
- Test: `crates/driver-core/src/manager.rs` test module

**Interfaces:**
- Consumes: `status_of` (Task 9).
- Produces: `impl DeviceManager { pub fn status(&self) -> CutStatus }`.
- Note: `snapshot()` stays for now so the existing tests keep compiling; Task 14 removes it.
- **`DeviceEvent` does not gain its `status` field here.** `apps/desktop/src/device.rs`'s `progress_event_marks_cache_transmitting` test builds `DeviceEvent` as a struct literal, so a new required field breaks `apps/` — which this task may not touch. Task 12 adds the field, because it deletes that test anyway (it covers the `Transmitting` synthesis that Task 12 removes).
- `status()` must not outlive its worker: a panicked worker would otherwise leave the published cell frozen on `Sending` with a live cancel button, and after Task 14 there is no other read path to notice.

- [ ] **Step 1: Write the failing test**

```rust
    /// The desktop cannot use `snapshot()` — it round-trips through the worker and
    /// blocks mid-transmit, which is why `apps/desktop/src/device.rs` grew a second
    /// cache. `status()` reads published memory instead, so it answers while the
    /// worker is busy.
    #[test]
    fn status_answers_while_the_worker_is_transmitting() {
        let gate = Arc::new(AtomicBool::new(false));
        let factory = Arc::new(GateFactory { gate: gate.clone() });
        let (mgr, _events) = DeviceManager::spawn(factory);
        mgr.connect(test_info()).expect("connect");
        std::thread::spawn({
            let mgr_passes = vec![CutPass { job: big_job() }];
            move || { let _ = mgr.cut(mgr_passes); }
        });
        // The worker is parked inside a write; `status()` must still return.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let s = mgr.status();
            if s.phase == Phase::Sending {
                assert!(s.actions.cancel, "a sending cut can be cancelled");
                break;
            }
            assert!(Instant::now() < deadline, "status() never reported Sending");
        }
        gate.store(true, Ordering::SeqCst);
    }
```

Reuse the existing `GateTransport` harness (`manager.rs:1021`) for `GateFactory`; if it is not already shaped as a factory, add one beside it following `TestFactory` at `manager.rs:715`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p driver-core status_answers_while_the_worker_is_transmitting`
Expected: FAIL — `no method named 'status' found`.

- [ ] **Step 3: Write minimal implementation**

Add the published cell to `DeviceManager` and the worker:

```rust
pub struct DeviceManager {
    cmd_tx: mpsc::SyncSender<Command>,
    handle: thread::JoinHandle<()>,
    cancel_flag: Arc<AtomicBool>,
    /// Published by the worker on every state change and progress tick. Read
    /// without touching the command channel, so a caller is never blocked by a
    /// busy worker. It may lag the worker by one event — that is the single
    /// documented lag rule, and every caller shares it.
    status: Arc<Mutex<CutStatus>>,
}

impl DeviceManager {
    /// Where the cut has got to, and what may be done next. Never blocks.
    pub fn status(&self) -> CutStatus {
        self.status.lock().unwrap().clone()
    }
}
```

In `spawn`, build `let status = Arc::new(Mutex::new(status_of(&DeviceState::Disconnected, 0)));`, clone it into the worker, and store it on the returned manager.

In the worker, wherever a `DeviceEvent` is emitted, publish first and attach the status to the event:

```rust
/// Publishes the status, then emits. Publishing first means a caller woken by an
/// event and calling `status()` cannot see an older value than the event carries.
fn emit(&mut self, event_tx: &mpsc::Sender<DeviceEvent>, job_id: u64, kind: DeviceEventKind) {
    let status = status_of(&self.state, self.total_passes);
    *self.status.lock().unwrap() = status.clone();
    let _ = event_tx.send(DeviceEvent { job_id, kind, status });
}
```

`total_passes` is the length of the submitted `Vec<CutPass>`; store it on the worker when a `Cut` command is accepted and reset it to 0 when the job ends.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p driver-core`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/driver-core/src/manager.rs
git commit -m "Publish the cut status so reading it does not wait on the worker"
```

---

### Task 11: Rework the manager's tests onto CutStatus

**Files:**
- Modify: `crates/driver-core/src/manager.rs` (tests at 661-1301)
- Test: same file

**Interfaces:**
- Consumes: `DeviceManager::status`, `Phase`, `CutStatus` (Tasks 9-10).
- Changes nothing public. `DeviceState` and `snapshot()` stay for now — Task 14 closes them once the last caller has converted, so every commit in between builds and the workspace stays green throughout.

Every scenario the deleted TypeScript tests covered must exist here as a Rust test — written failing first. Those are: a stale job's events are filtered; `Cancelled` is terminal; and lifecycle events (`job_id` 0) are accepted again once a job is released.

- [ ] **Step 1: Write the failing tests**

```rust
    /// Was `acceptEvent` in apps/desktop/ui/src/cut/viewmodel.ts. The filtering
    /// belongs here, so a caller does not track job ids at all.
    #[test]
    fn events_from_a_finished_job_do_not_reopen_it() {
        let (mgr, events) = DeviceManager::spawn(Arc::new(ReadyFactory));
        mgr.connect(test_info()).expect("connect");
        let job = mgr.cut(vec![CutPass { job: small_job() }]).expect("cut");
        let seen: Vec<DeviceEvent> = events.try_iter().collect();
        assert!(seen.iter().all(|e| e.job_id == job || e.job_id == 0),
                "no event may carry a foreign job id");
        assert_eq!(mgr.status().phase, Phase::Idle, "the job is over");
    }

    /// Was `terminalTransition`'s Cancelled case. Cancelled arrives as a state, not
    /// as a terminal event kind, and a caller must not have to know that.
    #[test]
    fn a_cancelled_job_reports_done_and_allows_another_cut() {
        let (mgr, _events) = DeviceManager::spawn(Arc::new(ReadyFactory));
        mgr.connect(test_info()).expect("connect");
        let _ = mgr.cut(vec![CutPass { job: big_job() }]);
        mgr.cancel();
        let s = wait_for_phase(&mgr, Phase::Done);
        assert!(s.actions.cut, "another cut is legal after a cancel");
    }

    /// Was the `NO_JOB` release case. Lifecycle events use job_id 0 and must remain
    /// visible after a job ends, or a reconnect goes unnoticed.
    #[test]
    fn lifecycle_events_survive_a_finished_job() {
        let (mgr, events) = DeviceManager::spawn(Arc::new(ReadyFactory));
        mgr.connect(test_info()).expect("connect");
        let _ = mgr.cut(vec![CutPass { job: small_job() }]);
        let _ = events.try_iter().count();
        mgr.disconnect().expect("disconnect");
        let after: Vec<DeviceEvent> = events.try_iter().collect();
        assert!(after.iter().any(|e| e.job_id == 0), "disconnect must be reported");
        assert_eq!(mgr.status().phase, Phase::Disconnected);
    }
```

Add the helper the suite needs, replacing `wait_for_state` (`manager.rs:872`):

```rust
    /// Polls `status()` — which never blocks — until the phase arrives.
    fn wait_for_phase(mgr: &DeviceManager, want: Phase) -> CutStatus {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let s = mgr.status();
            if s.phase == want { return s; }
            assert!(Instant::now() < deadline, "never reached {want:?}, last was {:?}", s.phase);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p driver-core`
Expected: FAIL — `wait_for_phase` / `Phase` not in scope in the test module until imported; then the three new tests fail on assertions if the filtering is not already correct.

- [ ] **Step 3: Rework the suite**

1. Leave `DeviceState` and `snapshot()` exactly as they are; Task 14 removes them.
2. Replace every `mgr.snapshot()` assertion in the test module with a `mgr.status()` assertion on `phase`/`actions`/`pass`/`sent`.
3. Replace the `WRITE_CHUNK` assertions (`manager.rs:1083`, `:1119`, `:1136`, `:1143`) with assertions on `status().sent` — the byte progression is now observable through the interface, so the private constant no longer needs to be reachable from a test:

```rust
        // Was: assert_eq!(written.iter().filter(|&&b| b == 0xAA).count(), 2 * WRITE_CHUNK - 2);
        let s = wait_for_phase(&mgr, Phase::Sending);
        let progress = s.sent.expect("a sending cut reports bytes");
        assert!(progress.sent > 0 && progress.sent < progress.total, "partial progress: {progress:?}");
```

4. Keep all nine test doubles (`FakeDriver`, `TestFactory`, `FlakyOpenFactory`, `TeeTransport`, `ReadyFactory`, `PumaFactory`, `probe_factory`, `GateTransport`, `ScriptedFactory`) — they implement `Driver`/`Transport`, which is a legitimate adapter at a real seam, not a reach past the interface.

- [ ] **Step 4: Run the suite**

Run: `cargo test --workspace --locked`
Expected: PASS. Nothing public changed, so every caller still builds.

- [ ] **Step 5: Commit**

```bash
git add crates/driver-core/src/manager.rs
git commit -m "Test the device manager through the status it reports"
```

---

### Task 12: Delete the desktop's parallel state store

**Files:**
- Modify: `apps/desktop/src/device.rs` (delete the cache at 46-59 and `record_state` at 101-114, delete `is_active` at 223-226), `apps/desktop/src/main.rs` (bridge at 86-100, close guard at 66-76), `apps/desktop/src/ipc.rs:109-112`
- Test: `apps/desktop/src/device.rs` test module

**Interfaces:**
- Consumes: `DeviceManager::status`, `CutStatus::is_active` (Tasks 9-10).
- Produces: `ipc::get_device_state` returns `CutStatus`; `DeviceManagerHandle::status()` replaces `cached_state()`; `DeviceEvent` gains `pub status: CutStatus`.

**Also required here, or Task 14 cannot close the surface:** `DeviceEventKind::StateChanged(DeviceState)` carries the internal enum in its payload, so `DeviceState` stays publicly reachable through the event stream no matter what Task 14 does to the type's visibility. Once `DeviceEvent` carries a `CutStatus`, that payload is redundant — drop it to a unit `StateChanged` variant and let the attached status say what changed. Task 11 confirmed three tests still name `DeviceState` solely through this payload; they convert with it.

**Carried over from Task 10:** add `pub status: CutStatus` to `DeviceEvent` in `crates/driver-core/src/manager.rs` here. Task 10 could not, because `progress_event_marks_cache_transmitting` in `apps/desktop/src/device.rs` builds the struct literally — and that test covers the `Transmitting` synthesis this task deletes, so it goes away in the same change. The field is what lets the UI render from the event it just received instead of polling after it.

**One ordering wrinkle to respect, not to fix here:** the worker emits `JobComplete` before the state becomes `Idle`, so the status attached to that event still reads `Sending`. Terminal-ness therefore comes from `phase` reaching `Done`/`Idle` on a later status, never from the event kind. Task 13's UI must key on phase for that reason.

- [ ] **Step 1: Write the failing test**

```rust
    /// The bridge used to synthesize `Transmitting` from `Progress` because the
    /// worker never re-emitted a state mid-transmit. `CutStatus` carries progress in
    /// the phase itself, so the synthesis has nothing left to do.
    #[test]
    fn status_reports_sending_without_the_bridge_synthesizing_it() {
        let (handle, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        handle.connect(test_info()).expect("connect");
        let _ = handle.cut_from_request(&AppState::new(), test_request());
        let s = handle.status();
        assert!(matches!(s.phase, Phase::Sending | Phase::AwaitingConfirmation | Phase::Done),
                "unexpected phase: {:?}", s.phase);
        assert_eq!(s.is_active(), s.phase != Phase::Done);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p desktop status_reports_sending`
Expected: FAIL — `no method named 'status'` on `DeviceManagerHandle`.

- [ ] **Step 3: Implement**

1. In `device.rs`, delete the `Mutex<DeviceState>` cache and `record_state`; add:

```rust
    /// Where the cut has got to. Reads `driver-core`'s published status, which
    /// never blocks on the worker — so the window-close handler and the IPC
    /// command can both call it freely.
    pub fn status(&self) -> CutStatus {
        match self.manager.lock().unwrap().as_ref() {
            Some(mgr) => mgr.status(),
            None => CutStatus::disconnected(),
        }
    }
```

Add `CutStatus::disconnected()` to `status.rs` as `status_of(&DeviceState::Disconnected, 0)` exposed publicly, since the desktop needs a value when no manager exists.

2. Delete `is_active` (`device.rs:223-226`); the close guard in `main.rs:66-76` becomes:

```rust
                if dev.status().is_active() {
                    api.prevent_close();
                    window.emit("cut-in-progress", ()).ok();
                }
```

3. The bridge in `main.rs:86-100` keeps only the 10 Hz `Progress` coalescing and the `emit`; drop the `record_state` call.

4. `ipc::get_device_state` returns `Result<CutStatus, IpcError>`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p desktop`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src
git commit -m "Read the cut status from one place instead of keeping a second copy"
```

---

### Task 13: Render the dialog from CutStatus and delete the TypeScript state machine

**Files:**
- Modify: `apps/desktop/ui/src/ipc.ts:84-109`, `apps/desktop/ui/src/cut/viewmodel.ts` (delete lines 142-281's phase/action/event logic), `apps/desktop/ui/src/cut/CutDialog.tsx`, `apps/desktop/ui/src/App.tsx:158-242`
- Modify: `apps/desktop/ui/src/cut/viewmodel.test.ts` (delete five `describe` blocks)
- Modify: `apps/desktop/ui/e2e/smoke.spec.ts`

**Interfaces:**
- Consumes: the `CutStatus` JSON shape from Task 9 (`camelCase`, `phase` as a bare string, `actions` as four booleans).
- Produces: `export type CutStatus` in `ipc.ts`. Removes `DeviceState`, `DevicePhase`, `dialogPhase`, `canStartCut`, `dialogButtons`, `acceptEvent`, `terminalTransition`.

- [ ] **Step 1: Delete the tests for the deleted logic**

Remove these `describe` blocks from `viewmodel.test.ts`: `acceptEvent` (232-249), `terminalTransition` (250-307), `dialogPhase` (408 onward), plus the `canStartCut` and `dialogButtons` blocks. Keep `reorderPass` (19-99) and `toCutRequest` (308-407). Leave `effectiveSettings` and `fieldDisabled` alone — they belong to review candidate 2, which is not in this plan.

- [ ] **Step 2: Run the suite to confirm what breaks**

Run: `npm --prefix apps/desktop/ui test`
Expected: PASS (the remaining tests do not touch the deleted helpers). If something fails, a keeper was importing a deleted helper — fix the import, not the test.

- [ ] **Step 3: Replace the type and the rendering**

In `ipc.ts`, replace the `DeviceState` union with:

```typescript
export type Phase =
  | "Disconnected" | "Connecting" | "Disconnecting" | "Idle" | "Sending"
  | "AwaitingConfirmation" | "AwaitingColorSwap" | "Cancelling" | "Done" | "Failed";

/** Mirrors driver_core::CutStatus. The phase says where the cut is; actions say
 *  which buttons are legal. Nothing here needs interpreting. */
export type CutStatus = {
  phase: Phase;
  actions: { cut: boolean; cancel: boolean; resume: boolean; confirm: boolean };
  pass: { index: number; total: number } | null;
  sent: { sent: number; total: number } | null;
  error: unknown | null;
};
```

In `CutDialog.tsx`, drive the buttons straight off `status.actions` — `disabled={!status.actions.resume}` and so on — and the wording off `status.phase`. In `App.tsx`, the `device-event` listener stores `event.status` and nothing else; delete `jobIdRef` and the `acceptEvent`/`terminalTransition` calls.

- [ ] **Step 4: Update the Playwright mock**

In `smoke.spec.ts`, replace the per-pass state machine (233-259) and the emitted `StateChanged` payloads with `CutStatus` values, and replace `get_device_state` with a `CutStatus`. Keep every existing assertion. The `__test_fail_next_resume` hook (337-342) stays.

- [ ] **Step 5: Build, test, commit**

```bash
npm --prefix apps/desktop/ui run build
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run e2e
git add apps/desktop/ui
git commit -m "Render the cut dialog from the status it is given, not from a copy of the state machine"
```

---

### Task 14: Point the CLI loop at CutStatus, then close the surface

**Files:**
- Modify: `crates/cli/src/cut.rs` (the loop from Task 4)
- Modify: `crates/driver-core/src/manager.rs` (`DeviceState` visibility, delete `snapshot`)
- Test: `crates/cli/tests/plain_cut.rs` (existing tests must still pass)

**Interfaces:**
- Consumes: `driver_core::{CutStatus, Phase}`.
- Removes: the CLI's last use of `DeviceState`; then `DeviceManager::snapshot` and `DeviceState`'s public visibility (`pub` → `pub(crate)`).
- Keeps public: `DeviceError`, `DeviceEvent`, `DeviceEventKind`, `CutPass`.

This is the last caller, so the surface closes here — every commit before this one builds, and this one both converts the final caller and makes the old surface unreachable.

- [ ] **Step 1: Rewrite the loop**

```rust
    loop {
        let status = mgr.status();
        match status.phase {
            Phase::AwaitingColorSwap => {
                let at = status.pass.map(|p| p.index + 1).unwrap_or(1);
                let prompt = format!("Pass {at}/{total}: swap tool, press Enter to resume");
                if operator.wait_ack(&prompt, &mgr) {
                    mgr.resume().map_err(|e| format!("resume: {e:?}"))?;
                }
            }
            Phase::AwaitingConfirmation => {
                let at = status.pass.map(|p| p.index + 1).unwrap_or(1);
                let prompt = format!("Pass {at}/{total} cutting; press Enter once the machine finishes");
                if operator.wait_ack(&prompt, &mgr) {
                    mgr.confirm_pass_done().map_err(|e| format!("confirm: {e:?}"))?;
                }
            }
            Phase::Idle | Phase::Done => {
                println!("done: {total} passes cut");
                return Ok(());
            }
            Phase::Failed => return Err(format!("device error: {:?}", status.error)),
            // Sending / Cancelling / connection phases: nothing for the operator to do.
            _ => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
```

`wait_for_enter_or_cancel` checks `mgr.status().phase == Phase::Done` instead of matching `Cancelled`.

- [ ] **Step 2: Run the suite**

Run: `cargo test --workspace --locked`
Expected: PASS.

- [ ] **Step 3: Close the old surface**

In `crates/driver-core/src/manager.rs`, change `pub enum DeviceState` to
`pub(crate) enum DeviceState`, and narrow `DeviceManager::snapshot` to `pub(crate)` rather
than deleting it. Nothing outside `driver-core` should reference either any more — if the
build fails here, a caller was missed in Tasks 11-13 and that is exactly what this step is
for.

**Why narrow rather than delete.** `assert_cancel_mid_transmit` reads `completion_known`
through `snapshot()`, and that field has no `CutStatus` equivalent — Task 12 removed the
event trail that used to carry it, so this is now its only observation point. Tests inside
`manager.rs` are in-crate, so a `pub(crate)` `snapshot()` keeps that assertion working while
the public surface closes exactly as intended. Deleting it would force either a widened
`CutStatus` or the loss of the assertion, and neither is worth it. If you find `snapshot()`
has no remaining in-crate caller, then delete it.

- [ ] **Step 4: Prove the state machine is not reachable outside driver-core**

Run: `cargo test --workspace --locked`
Expected: PASS.

Run: `grep -rn "DeviceState\|AwaitingCompletion\|WaitingForColorSwap\|CancelRequested" crates/cli/src apps/desktop/src apps/desktop/ui/src`
Expected: no matches.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/cut.rs crates/driver-core/src/manager.rs
git commit -m "Drive the CLI cut from the reported status, and shut the state machine in"
```

---

### Task 15: Record CutStatus in the domain language and the checklist

**Files:**
- Modify: `CONTEXT.md` (new entry after **Pass**)
- Modify: `apps/desktop/MANUAL-CHECKLIST.md`

- [ ] **Step 1: Add the glossary entry**

```markdown
**CutStatus**:
Where a cut has got to and what the operator may do next — the phase it is in, how the last one
ended, which of cancel/resume/confirm are legal, which Pass of how many, and how many bytes of it
have been sent. The only thing a Driver's caller is told about a cut; the states behind it are not
anybody else's business, and a caller that keeps its own memory of them has gone wrong.
_Avoid_: device state, state machine, progress

**Phase**:
What a machine is doing right now — idle, sending, cancelling, awaiting an operator, or failed.
Says nothing about how a previous cut turned out; that is what an Ended is for.
_Avoid_: status, state

**Ended**:
How the last cut finished — completed or cancelled — or nothing at all if none has run since the
machine was connected. Separate from Phase because a finished machine and an untouched one are both
idle, and every caller that had to tell them apart invented its own memory to do it.
_Avoid_: result, outcome, done
```

Use the shape the code actually landed on: `CutStatus { phase, ended, actions, pass, sent, error }`,
`Phase` without a `Done` variant, `Ended { Completed, Cancelled }`. Read
`crates/driver-core/src/status.rs` and match it — a glossary that disagrees with the type is worse
than none.

- [ ] **Step 2: Add the checklist items**

```markdown
## CutStatus (architecture review candidate 1)

- [ ] Cut dialog buttons match what the machine allows at each stage — nothing enabled that errors when pressed.
- [ ] Progress advances during a pass; pass n of m is correct across a colour swap.
- [ ] Cancel mid-cut lands on Done and a second cut can be started straight after.
- [ ] Unplug mid-cut shows the failure with its reason, and the dialog offers no dead buttons.
- [ ] Closing the window mid-cut still prompts.
- [ ] `cuthulhu cut --by-color` on the Puma still prompts per pass and completes.
```

- [ ] **Step 3: Commit**

```bash
git add CONTEXT.md apps/desktop/MANUAL-CHECKLIST.md
git commit -m "Name the value a cut reports, and list what a human still has to watch"
```

---

---

### Task 16: Say how a cut ended, instead of making callers infer it

**Files:**
- Modify: `crates/driver-core/src/status.rs` (add `Ended`, add `CutStatus::ended`, delete `Phase::Done`, update `status_of` and its tests)
- Modify: `crates/driver-core/src/manager.rs` (the worker must remember how the last job ended)
- Modify: `crates/cli/src/cut.rs` (terminal messages come from `ended`)
- Modify: `apps/desktop/ui/src/ipc.ts` (type), `apps/desktop/ui/src/cut/CutDialog.tsx` (delete the latch), `apps/desktop/ui/e2e/smoke.spec.ts` (mock), `apps/desktop/ui/dist` (rebuild)
- Test: `crates/driver-core/src/status.rs`, `crates/driver-core/src/manager.rs`, `crates/cli/tests/plain_cut.rs`, `apps/desktop/ui/e2e/smoke.spec.ts`

**Interfaces:**
- Consumes: everything Tasks 9-14 built.
- Produces:
  ```rust
  pub enum Ended { Completed, Cancelled }
  // CutStatus gains: pub ended: Option<Ended>
  // Phase loses: Done
  ```

**Why.** `Phase::Done` was reachable only from a cancelled job, because a job that finishes normally rests on `Idle`. So `Idle` meant three things — no cut yet, cut finished, freshly connected — and both callers had to invent memory to tell them apart: the dialog latched a bit off `actions.cancel`, and the CLI could not name a cancellation at all until it read `pass`/`sent` back out. That inference is the exact thing `CutStatus` exists to prevent, so the fix belongs in the status, not in each caller.

After this task `phase` means only "what is happening now", and `ended` means "how the last job finished". `Idle` with `ended: None` is a fresh connection; `Idle` with `ended: Some(Completed)` is a finished cut; `Idle` with `ended: Some(Cancelled)` is a cancelled one. No caller needs to remember anything.

- [ ] **Step 1: Write the failing tests in `status.rs`**

```rust
    /// The wart this task removes: a cut that finished and a cut that was cancelled both
    /// rested on `Idle`, so no caller could tell them apart without keeping its own
    /// memory of what it had seen.
    #[test]
    fn a_finished_cut_and_a_cancelled_one_are_distinguishable() {
        let fresh = status_of(&DeviceState::Idle, 0, None);
        assert_eq!(fresh.phase, Phase::Idle);
        assert_eq!(fresh.ended, None, "nothing has run yet");

        let finished = status_of(&DeviceState::Idle, 3, Some(Ended::Completed));
        assert_eq!(finished.phase, Phase::Idle, "phase says what is happening now");
        assert_eq!(finished.ended, Some(Ended::Completed));

        let cancelled = status_of(
            &DeviceState::Cancelled { job_id: 1, pass_index: 1, submitted_bytes: 40, completion_known: false }, 3);
        assert_eq!(cancelled.phase, Phase::Idle, "a cancelled job is no longer in flight");
        assert_eq!(cancelled.ended, Some(Ended::Cancelled));
        assert_eq!(cancelled.pass, Some(PassPosition { index: 1, total: 3 }));
        assert!(cancelled.actions.cut, "another cut is legal after a cancel");
    }
```

Adapt the signature to however you thread the remembered outcome — the point is that `status_of` can report it, not the exact parameter list. Every existing test in `status.rs` that names `Phase::Done` must be updated in this task.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p driver-core status`
Expected: FAIL — `Ended` does not exist, `CutStatus` has no `ended`.

- [ ] **Step 3: Implement in `driver-core`**

Add `Ended` and `CutStatus::ended`; delete `Phase::Done` and map `DeviceState::Cancelled` to `Phase::Idle` with `ended: Some(Ended::Cancelled)`. The worker remembers how the last job ended: set `Completed` where it currently transitions to `Idle` after a job, set `Cancelled` on the cancel path, and clear it to `None` when a new `Cut` command is accepted so a fresh cut does not report the previous outcome.

- [ ] **Step 4: Simplify both callers**

CLI (`crates/cli/src/cut.rs`): the terminal arm becomes one `Phase::Idle` arm that reads `ended` — `Some(Cancelled)` prints the cancelled wording with pass and bytes, anything else prints the completed wording. Delete the `Phase::Done` arm.

Dialog (`apps/desktop/ui/src/cut/CutDialog.tsx`): **delete the latch entirely.** The banner reads `status.ended` for a finished or cancelled outcome and `status.phase === "Failed"` for a failure. No `useState`, no `useEffect` reconstructing an outcome, no reading `actions.cancel` as a liveness bit. This deletion is the point of the task — if the latch survives in any form, the task is not done.

- [ ] **Step 5: Update the mock and the types**

`ipc.ts`: drop `"Done"` from the `Phase` union, add `ended: "Completed" | "Cancelled" | null`. `smoke.spec.ts`: emit `ended` where the mock rests after a job, and add a case asserting a normally-completed cut reports completion — the mock previously could not express one.

- [ ] **Step 5a: Make the terminal wording reachable from a test**

Task 14 could only unit-test its terminal messages against a helper, because three things block an
end-to-end test of `run`, all verified: the wording leaves via `println!` so nothing in-process can
read it; `ctrlc::set_handler` is process-wide and errors on a second call, so no test binary can call
`run` twice; and `run` owns its `DeviceManager` with no handle for a test to cancel through. Fix the
first two here, since this task is rewriting that arm anyway:

- `run` returns its outcome instead of printing it — `Ok(Outcome)` where `Outcome` names the ending
  and carries what the message needs (`Completed { passes }`, `Cancelled { pass, sent }`). `main.rs`
  does the printing. `ended_message` becomes a function of `Outcome`, and the tests move onto the
  real loop rather than a helper beside it.
- Move `ctrlc::set_handler` out of `run` and into `main.rs`. Installing a process-wide signal handler
  is not a library function's business, and while it lives in `run` no test binary can drive the loop
  more than once. `run` takes whatever cancel handle it needs from its caller.

Then add the end-to-end test Task 14 could not write: a multi-pass `Unattended` cut, cancelled
mid-job, asserting the returned outcome is the cancelled one. Leave the third blocker (`run` owning
its manager) alone if the cancel handle above is enough to drive it.

- [ ] **Step 6: Verify everything**

```bash
cargo test --workspace --locked
npm --prefix apps/desktop/ui run build
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run e2e
grep -rn "Phase::Done\|\"Done\"" crates apps/desktop/src apps/desktop/ui/src   # expect nothing
```

- [ ] **Step 7: Commit**

```bash
git add crates/driver-core/src/status.rs crates/driver-core/src/manager.rs crates/cli/src/cut.rs \
        crates/cli/tests/plain_cut.rs apps/desktop/ui/src apps/desktop/ui/e2e apps/desktop/ui/dist
git commit -m "Report how a cut ended, so no caller has to remember"
```

---

## Self-review

**Coverage against the agreed decisions.** Plain path promotes unstroked paths at import (Task 1) and stays exactly one pass (Tasks 1-2); it goes through `plan_cut` so preflight runs (Task 2); it transmits via `DeviceManager` with the registry's `open_transport` and a TTY guard (Tasks 4, 7); `check_out_of_bounds_scope` is deleted and the colour flags error (Task 3); the cut function takes a factory parameter and is tested over `MockTransport` (Tasks 4-5); `build_bytes` and its test are deleted (Task 6). `CutStatus` carries phase, actions, pass position and bytes, and is both queried and carried on events (Tasks 9-10); `DeviceState` goes private (Task 11); the worker publishes so reads never block, deleting the desktop cache and the `Transmitting` synthesis (Tasks 10, 12); all three callers convert (Tasks 12-14); 10 Hz coalescing stays in the bridge (Task 12); manager tests assert `CutStatus`, the `WRITE_CHUNK` reach-past dies, and the three TypeScript scenarios become Rust tests (Task 11); the five TypeScript blocks are deleted while `reorderPass` and `toCutRequest` survive (Task 13). `plan_passes` is untouched throughout and #68 stays open.

**Two additions beyond the agreed shape, both forced by the code.** `Phase` includes `Disconnected`/`Connecting`/`Disconnecting` because the dialog maps them today. `plan_plain_cut` needs an explicit empty-document check, or an SVG with no paths reports `UnknownPassColor` instead of "no cuttable paths".

**Sequencing.** `status()` is added alongside the existing surface (Task 10), every caller converts while both compile (Tasks 11-13), and `DeviceState` goes private in the same task that converts the last caller (Task 14). Every commit builds and `cargo test --workspace --locked` passes at every task boundary, so the history stays bisectable. The branch never lands with both surfaces public, which is what the single-change decision was about.

**Two plan-vs-rubric conflicts resolved before execution.** Task 4 ships an untested extraction by design — Task 5 is its test, and a reviewer should not treat the absence as a finding. Task 6's test asserts a refusal that does not happen today (off-bed geometry printing bytes through `build_bytes`), so it is genuinely red before the deletion and green after.

**Not in this plan.** Review candidate 2 (resolved `Settings` and `MachineCaps` across the seam) and its `effectiveSettings`/`fieldDisabled` deletions. Candidates 4-10. Issue #68.
