<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Cut refusal `Display` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `CutError` and `PreflightError` one operator-facing `Display` in `cutplan`, so the desktop and the CLI stop writing their own English for the same ten refusals — and the CLI stops printing `Debug` output for four of them.

**Architecture:** Copy the shape `TraceError` already uses in this workspace (`crates/trace/src/lib.rs:171-200`): `Display` for the sentence, `std::error::Error` for the trait bound, `code() -> &'static str` for the caller that must branch on the kind of failure rather than match its text. `CutError` delegates both to `PreflightError` for its wrapped variant. `apps/desktop/src/device.rs` collapses to one line that pairs `code()` with `to_string()`; `crates/cli/src/pipeline.rs` keeps two arms and defers the other eight.

**Tech Stack:** Rust 2021, workspace crates `cutplan` / `cli` / `apps/desktop`. No new dependencies — `thiserror` is not in this workspace and `TraceError` hand-writes its impl.

**Spec:** `docs/superpowers/specs/2026-07-29-cut-refusal-display-design.md`

## Global Constraints

- **SPDX header on every file** — `// SPDX-License-Identifier: GPL-3.0-or-later`. All files here already have one; do not remove it.
- **`cargo test --workspace --locked`** is the gate. `--locked` is mandatory; this change adds no dependency, so `Cargo.lock` must not change.
- **`CONTEXT.md` is normative vocabulary.** Relevant here: *Preflight* (not "validation"), *Stale plan* (not "revision mismatch"), and the `_Avoid_` entry for **enabled pass** — a colour nobody lists in `PlanOptions::passes` is simply not cut, so no refusal text may name an enable flag.
- **Comments explain why, not what.** Every comment specified below documents a constraint or a trap. Do not add comments that restate the code.
- **No `ui/dist` or Node work.** This change is Rust-only; no `apps/desktop/ui` file is touched, so no `npm run build` and no `dist/` commit.
- **Codes are the stable half of the contract.** Every `code()` string below is the exact string `apps/desktop/src/device.rs` emits today. `apps/desktop/ui/src/cut/CutDialog.tsx:165` branches on `"stale_plan"`. Changing a code breaks the frontend silently; changing a message does not.

## File Structure

| File | Responsibility after this change |
|---|---|
| `crates/cutplan/src/preflight.rs` | Owns the seven preflight rules **and** what each refusal says to an operator. Gains `impl Display`, `impl Error`, `impl PreflightError { fn code }` and one table test. |
| `crates/cutplan/src/plan.rs` | Owns `plan_cut` and the three top-level refusals. Gains the same three impls, delegating the `Preflight` variant, plus one table test. |
| `apps/desktop/src/device.rs` | Keeps knowing that IPC needs a code; stops knowing what any refusal means. Two functions become one. |
| `crates/cli/src/pipeline.rs` | Keeps the two sentences only it can write (an SVG was imported; `--allow-out-of-bounds` exists); defers the other eight. |

---

### Task 1: `PreflightError` says what each rule refused

**Files:**
- Modify: `crates/cutplan/src/preflight.rs:12-21` (add impls directly below the enum)
- Test: `crates/cutplan/src/preflight.rs` (in the existing `mod tests`, at the end)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `impl std::fmt::Display for PreflightError`, `impl std::error::Error for PreflightError`, and `PreflightError::code(&self) -> &'static str`. Task 2 delegates to both; Tasks 3 and 4 reach them through `CutError`.

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `crates/cutplan/src/preflight.rs`:

```rust
    /// Both the whole table at once: a new variant fails to compile the match in
    /// `Display`/`code`, and a reworded one fails here. These strings are what an
    /// operator reads, so they are worth pinning.
    #[test]
    fn every_refusal_has_a_sentence_and_a_code() {
        let cases: Vec<(PreflightError, &str, &str)> = vec![
            (
                PreflightError::NothingToCut,
                "nothing_to_cut",
                "no pass selected for this cut has any geometry",
            ),
            (
                PreflightError::NonFiniteGeometry(NodeId(3)),
                "non_finite_geometry",
                "shape #3 has a coordinate that is not a finite number",
            ),
            (
                PreflightError::DegeneratePolyline(NodeId(4)),
                "degenerate_polyline",
                "shape #4 has a path with fewer than two points",
            ),
            (
                PreflightError::OutOfBounds { node: NodeId(5), bounds: (0.0, 0.0, 304.8, 304.8) },
                "out_of_bounds",
                "shape #5 lies outside the 304.8 x 304.8 mm cutting area",
            ),
            (
                PreflightError::SettingsOutOfRange("speed must be 1..=30"),
                "settings_out_of_range",
                "speed must be 1..=30",
            ),
            (
                PreflightError::MachineMismatch { document: "puma".into(), device: "cameo5".into() },
                "machine_mismatch",
                "this document is set up for `puma`, but the connected machine is `cameo5`",
            ),
            (
                PreflightError::OutputTooLarge(80_000_000),
                "output_too_large",
                "the encoded cut is about 76 MB, over the 64 MB limit",
            ),
        ];
        for (error, code, message) in cases {
            assert_eq!(error.code(), code, "code for {error:?}");
            assert_eq!(error.to_string(), message, "message for {error:?}");
        }
    }

    /// No refusal may print `Debug` at an operator. The four the CLI used to leak
    /// (`preflight: MachineMismatch { .. }`) are the reason this type gained `Display`.
    #[test]
    fn no_refusal_leaks_debug_formatting() {
        let errors = [
            PreflightError::NothingToCut,
            PreflightError::NonFiniteGeometry(NodeId(1)),
            PreflightError::DegeneratePolyline(NodeId(1)),
            PreflightError::OutOfBounds { node: NodeId(1), bounds: (0.0, 0.0, 100.0, 100.0) },
            PreflightError::SettingsOutOfRange("force must be 1..=33"),
            PreflightError::MachineMismatch { document: "puma".into(), device: "cameo5".into() },
            PreflightError::OutputTooLarge(80_000_000),
        ];
        for error in errors {
            let message = error.to_string();
            assert!(!message.contains('{'), "{message}");
            assert!(!message.contains("NodeId("), "{message}");
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cutplan every_refusal_has_a_sentence_and_a_code`

Expected: compile error — `no method named 'code' found for enum 'PreflightError'` and `'PreflightError' doesn't implement 'std::fmt::Display'`.

- [ ] **Step 3: Write the implementation**

Insert directly below the `PreflightError` enum (after `crates/cutplan/src/preflight.rs:21`):

```rust
/// What each rule refused, in the words an operator reads. It lives next to the rules
/// rather than in each caller because the desktop and the CLI used to write it twice,
/// and the CLI's copy fell through to `Debug` for four of these.
impl std::fmt::Display for PreflightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Not "no enabled pass": a colour nobody selected is simply not cut, so
            // there is no flag for an operator to go looking for (CONTEXT.md).
            PreflightError::NothingToCut =>
                write!(f, "no pass selected for this cut has any geometry"),
            PreflightError::NonFiniteGeometry(node) =>
                write!(f, "shape #{} has a coordinate that is not a finite number", node.0),
            PreflightError::DegeneratePolyline(node) =>
                write!(f, "shape #{} has a path with fewer than two points", node.0),
            // bounds is (0, 0, width_mm, height_mm) — the machine's area, not the shape's.
            PreflightError::OutOfBounds { node, bounds } =>
                write!(f, "shape #{} lies outside the {} x {} mm cutting area", node.0, bounds.2, bounds.3),
            // Already a whole clause naming the setting and its range, so a prefix would read twice.
            PreflightError::SettingsOutOfRange(message) => write!(f, "{message}"),
            PreflightError::MachineMismatch { document, device } =>
                write!(f, "this document is set up for `{document}`, but the connected machine is `{device}`"),
            // Megabytes, not the byte count the variant carries: the estimate weights 16
            // bytes per point by repeat_count, so printing it exactly claims a precision
            // it does not have. Divisor matches the rule's own `64 * 1024 * 1024`.
            PreflightError::OutputTooLarge(bytes) =>
                write!(f, "the encoded cut is about {} MB, over the 64 MB limit", bytes / (1024 * 1024)),
        }
    }
}
impl std::error::Error for PreflightError {}

impl PreflightError {
    /// Stable identifier for a caller that must branch on the *kind* of refusal rather
    /// than show its text — the desktop sends it as `IpcError::code`, and `CutDialog.tsx`
    /// keys off `stale_plan` instead of matching a sentence across a language boundary.
    pub fn code(&self) -> &'static str {
        match self {
            PreflightError::NothingToCut => "nothing_to_cut",
            PreflightError::NonFiniteGeometry(_) => "non_finite_geometry",
            PreflightError::DegeneratePolyline(_) => "degenerate_polyline",
            PreflightError::OutOfBounds { .. } => "out_of_bounds",
            PreflightError::SettingsOutOfRange(_) => "settings_out_of_range",
            PreflightError::MachineMismatch { .. } => "machine_mismatch",
            PreflightError::OutputTooLarge(_) => "output_too_large",
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cutplan preflight`

Expected: PASS, including the two new tests and the ~20 existing rule tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cutplan/src/preflight.rs
git commit -m "Say what each preflight rule refused, next to the rule that refused it

Two callers wrote their own English for all seven, and the CLI's copy fell
through to Debug for four — a document built for a Puma sent to a Cameo
printed a struct literal at the terminal."
```

---

### Task 2: `CutError` says the same, and delegates the wrapped half

**Files:**
- Modify: `crates/cutplan/src/plan.rs:57-63` (add impls directly below the enum)
- Test: `crates/cutplan/src/plan.rs` (in the existing `mod tests`, at the end)

**Interfaces:**
- Consumes: `PreflightError::code()` and `impl Display for PreflightError` from Task 1.
- Produces: `impl std::fmt::Display for CutError`, `impl std::error::Error for CutError`, and `CutError::code(&self) -> &'static str`. Task 3 calls both on one line; Task 4 calls `to_string()` on it.

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `crates/cutplan/src/plan.rs`:

```rust
    /// The three top-level refusals. Preflight's own table is pinned in preflight.rs;
    /// what matters here is that the wrapped variant adds no prefix of its own — a
    /// caller printing "preflight: ..." in front of a finished sentence reads twice.
    #[test]
    fn every_refusal_has_a_sentence_and_a_code() {
        let stale = CutError::StalePlan { expected: 7, actual: 9 };
        assert_eq!(stale.code(), "stale_plan");
        assert_eq!(stale.to_string(), "the document changed since this cut was planned");

        let unknown = CutError::UnknownPassColor(Some(0xFF0000FF));
        assert_eq!(unknown.code(), "unknown_pass_color");
        assert_eq!(unknown.to_string(), "no planned pass has color #FF0000FF");

        let colorless = CutError::UnknownPassColor(None);
        assert_eq!(colorless.code(), "unknown_pass_color");
        assert_eq!(colorless.to_string(), "no planned pass without a color");

        let wrapped = CutError::Preflight(PreflightError::NothingToCut);
        assert_eq!(wrapped.code(), "nothing_to_cut");
        assert_eq!(wrapped.to_string(), "no pass selected for this cut has any geometry");
    }
```

Add the import this test needs at the top of `mod tests`, next to the existing `use super::*;`:

```rust
    use crate::preflight::PreflightError;
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cutplan --lib plan::tests::every_refusal_has_a_sentence_and_a_code`

Expected: compile error — `no method named 'code' found for enum 'CutError'`.

- [ ] **Step 3: Write the implementation**

Insert directly below the `CutError` enum (after `crates/cutplan/src/plan.rs:63`):

```rust
/// Every refusal in the words an operator reads. `Preflight` forwards rather than
/// prefixing: its variants are already whole sentences, and the CLI's old
/// `format!("preflight: {e:?}")` is exactly the thing this replaces.
impl std::fmt::Display for CutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // The revision numbers are for a bug report, not for an operator; `Debug` keeps them.
            CutError::StalePlan { .. } =>
                write!(f, "the document changed since this cut was planned"),
            CutError::UnknownPassColor(Some(color)) =>
                write!(f, "no planned pass has color #{color:08X}"),
            // `plan_passes` only ever builds `Some(color)` passes, so this is a caller
            // asking for a pass that cannot exist rather than one that went missing.
            CutError::UnknownPassColor(None) =>
                write!(f, "no planned pass without a color"),
            CutError::Preflight(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for CutError {}

impl CutError {
    /// Stable identifier for a caller that must branch on the *kind* of refusal — see
    /// `PreflightError::code`, whose codes this passes through unchanged so the desktop
    /// keeps emitting one flat set across both types.
    pub fn code(&self) -> &'static str {
        match self {
            CutError::StalePlan { .. } => "stale_plan",
            CutError::UnknownPassColor(_) => "unknown_pass_color",
            CutError::Preflight(e) => e.code(),
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cutplan`

Expected: PASS — the whole crate, including Task 1's tests.

- [ ] **Step 5: Commit**

```bash
git add crates/cutplan/src/plan.rs
git commit -m "Give the chokepoint's own refusals a sentence, and forward preflight's

Preflight forwards rather than prefixing: its variants are already whole
sentences, so the CLI's `preflight: {e:?}` was a prefix on a struct literal."
```

---

### Task 3: The desktop keeps the code and drops the prose

**Files:**
- Modify: `apps/desktop/src/device.rs:197-231` (replace `map_cut_error` and `map_preflight_error` with one function)
- Modify: `apps/desktop/src/device.rs:4` (drop the now-unused `PreflightError` import)
- Test: no new test — the existing `prepare_cut` tests already assert codes, and `map_cut_error` no longer holds a decision.

**Interfaces:**
- Consumes: `CutError::code()` and `impl Display for CutError` from Task 2.
- Produces: nothing later tasks use.

- [ ] **Step 1: Confirm the existing tests pass before touching anything**

Run: `cargo test -p desktop`

Expected: PASS.

- [ ] **Step 2: Replace both functions with one**

Delete `map_cut_error` and `map_preflight_error` (`apps/desktop/src/device.rs:197-231`, from the `/// Every way plan_cut can refuse` doc comment through the closing brace of `map_preflight_error`) and put this in their place:

```rust
/// Every way `plan_cut` can refuse, as an IPC code the UI can branch on.
/// `stale_plan` is the one the frontend actually keys off (CutDialog.tsx).
/// The message is `cutplan`'s — this used to restate all ten here, and the CLI
/// restated them again, differently.
fn map_cut_error(e: CutError) -> IpcError {
    IpcError::new(e.code(), e.to_string())
}
```

- [ ] **Step 3: Drop the now-unused import**

In `apps/desktop/src/device.rs:4`, delete the whole line:

```rust
use cutplan::preflight::PreflightError;
```

- [ ] **Step 4: Run the tests and check for warnings**

Run: `cargo test -p desktop 2>&1 | tail -30`

Expected: PASS with no `unused import` warning. `map_cut_error` is still called at `device.rs:185`, so no dead-code warning either.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/device.rs
git commit -m "Carry cutplan's own words across IPC, and keep only the code

The dialog branches on the code; nothing matches a message body. What was
here restated all ten refusals, and its out-of-bounds wording was the worse
of the two copies — `node NodeId(3) outside (0.0, 0.0, 305.0, 305.0)`."
```

---

### Task 4: The CLI keeps the two sentences only it can write

**Files:**
- Modify: `crates/cli/src/pipeline.rs:175-190` (`describe_cut_error`)
- Test: `crates/cli/src/pipeline.rs` (in the existing `mod tests`, at the end)

**Interfaces:**
- Consumes: `impl Display for CutError` from Task 2 and `impl Display for PreflightError` from Task 1.
- Produces: nothing later tasks use.

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `crates/cli/src/pipeline.rs`:

```rust
    /// The leak this change exists to close: four preflight refusals used to fall
    /// through to `format!("preflight: {e:?}")`, so a document built for a Puma sent
    /// to a Cameo printed a struct literal. Tested against `describe_cut_error`
    /// directly because an SVG import never sets a machine id, so `plan_cut_from_svg`
    /// cannot reach `MachineMismatch`.
    #[test]
    fn a_machine_mismatch_reads_as_a_sentence() {
        let err = describe_cut_error(cutplan::CutError::Preflight(
            cutplan::preflight::PreflightError::MachineMismatch {
                document: "puma".into(),
                device: "cameo5".into(),
            },
        ));
        assert_eq!(err, "this document is set up for `puma`, but the connected machine is `cameo5`");
    }

    /// Out-of-bounds is the one refusal an operator may reasonably want to overrule,
    /// and only the CLI has a flag for it — the desktop hardcodes `allow_out_of_bounds:
    /// false`. So the shared sentence states the fact and this caller adds the escape.
    #[test]
    fn out_of_bounds_names_the_flag_that_overrules_it() {
        let err = describe_cut_error(cutplan::CutError::Preflight(
            cutplan::preflight::PreflightError::OutOfBounds {
                node: document::NodeId(3),
                bounds: (0.0, 0.0, 304.8, 304.8),
            },
        ));
        assert_eq!(
            err,
            "shape #3 lies outside the 304.8 x 304.8 mm cutting area — pass --allow-out-of-bounds to send it anyway",
        );
    }
```

`document` is already a dependency of this crate (`crates/cli/Cargo.toml:17`), so `document::NodeId(3)` resolves without adding anything.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cli a_machine_mismatch_reads_as_a_sentence`

Expected: FAIL with `assertion `left == right` failed`, left being `preflight: MachineMismatch { document: "puma", device: "cameo5" }`.

- [ ] **Step 3: Rewrite `describe_cut_error`**

Replace `crates/cli/src/pipeline.rs:175-190` (the doc comment through the closing brace) with:

```rust
/// `CutError` as something to print at a terminal. Two arms outlive the shared
/// `Display`: `NothingToCut`, because only this caller knows an SVG was imported
/// and that none of its paths were stroked; and out-of-bounds, because naming
/// `--allow-out-of-bounds` is the CLI's to do — the desktop hardcodes
/// `allow_out_of_bounds: false` and offers the operator no such control.
fn describe_cut_error(e: cutplan::CutError) -> String {
    use cutplan::preflight::PreflightError as P;
    match e {
        cutplan::CutError::Preflight(P::NothingToCut) => "no cuttable paths in SVG".into(),
        cutplan::CutError::Preflight(P::OutOfBounds { .. }) =>
            format!("{e} — pass --allow-out-of-bounds to send it anyway"),
        e => e.to_string(),
    }
}
```

The second arm reads `e` in its body. That compiles: `P::OutOfBounds { .. }` binds nothing, so the match does not move the scrutinee. If rustc disagrees, match on `&e` and adjust the last arm to `e => e.to_string()` over the reference.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cli`

Expected: PASS, including the two pre-existing assertions of `"no cuttable paths in SVG"` (`pipeline.rs:288` and `:349`) and the settings-range assertion at `:279`, none of which change.

- [ ] **Step 5: Run the whole workspace gate**

Run: `cargo test --workspace --locked`

Expected: PASS. `git status` must show no change to `Cargo.lock`.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/pipeline.rs
git commit -m "Print refusals the operator can act on, not the struct behind them

Four preflight refusals fell through to Debug here. Two sentences stay:
only this caller knows an SVG was imported, and only this caller has the
flag that overrules an out-of-bounds shape."
```

---

## Verification

After Task 4, confirm the change did what it claims rather than assuming it:

- [ ] `cargo test --workspace --locked` passes.
- [ ] `git diff --stat main...HEAD` shows four files and a net line reduction in the two callers.
- [ ] `grep -rn "{e:?}" crates/cli/src/pipeline.rs` returns nothing for `CutError` (the `plan: {e:?}` on `PlanError` at `:138` and `:164` is a different type and out of scope).
- [ ] `sed -n '210,212p' apps/desktop/src/device.rs | grep -n "nothing_to_cut\|machine_mismatch\|out_of_bounds"` returns nothing — `map_cut_error`'s body has no hand-written code string, only `e.code()`; the codes it forwards now come from `cutplan` and are pinned by Task 1's table test.
