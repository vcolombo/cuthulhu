<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Session framing in one place — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the `session_begin`/`pass_park`/`session_end` sequencing rule one home in `driver-core`, so `cuthulhu cut --dry-run` cannot drift from what `DeviceManager` actually transmits.

**Architecture:** Two free functions in `driver-core` — `open_pass` (prologue on the first Pass, then the encoded Pass) and `close_pass` (park between Passes, end the session after the last). The split is at the completion poll, because that is where the worker already breaks a Pass: it writes, waits for the machine, then writes again. The worker calls both across its two halves; the CLI's dry run concatenates them.

**Tech Stack:** Rust workspace, `cargo test --workspace --locked`. No new dependencies, no UI, no `ui/dist` rebuild.

Spec: `docs/superpowers/specs/2026-07-28-dry-run-framing-design.md`.

## Global Constraints

- **SPDX header on every file**: `// SPDX-License-Identifier: GPL-3.0-or-later`. Every file this plan touches already has one; do not disturb it.
- **`cargo test --workspace --locked`** is what CI runs. `--locked` is mandatory. This plan adds no dependency, so `Cargo.lock` must not change — if it does, something went wrong.
- **Comments explain why, not what.** Do not add restating comments. Where a step below carries a comment, that comment is part of the deliverable.
- **`CONTEXT.md` is normative vocabulary.** Use **Pass** (not run/cycle/layer), **Driver** (not backend/plugin). `CONTEXT.md:102` defines a Driver as what produces the bytes "to open, park and close a cutting session" — that is where `open_pass`/`close_pass` get their names.
- **Commit subjects are imperative with the reason attached**, e.g. "Share one device backend factory, since the reason for copying it was never true".
- **No AI attribution** in commits, comments, or documentation.
- Tasks 2 and 3 are behaviour-preserving. The bytes on the wire must be identical before and after; the existing tests are the proof, and any change to them is a signal the refactor went wrong.

---

### Task 1: `open_pass` and `close_pass` in `driver-core`

**Files:**
- Modify: `crates/driver-core/src/lib.rs` — add both functions after `write_all` (currently ends at `:76`), and a `Driver` fake plus two tests in the existing `#[cfg(test)] mod tests` (`:118-192`)

**Interfaces:**
- Consumes: `Driver`, `Job`, `DriverError` — all already in `crates/driver-core/src/lib.rs`.
- Produces:
  - `pub fn open_pass(d: &dyn Driver, job: &Job, index: usize) -> Result<Vec<u8>, DriverError>`
  - `pub fn close_pass(d: &dyn Driver, index: usize, total: usize) -> Vec<u8>`

  Both take `&dyn Driver`. The worker holds `&(dyn Driver + Send)`, which coerces — `probe_is_cutter` at `crates/driver-core/src/manager.rs:606` already relies on exactly that coercion, so no cast is needed at the call sites in Task 2.

- [ ] **Step 1: Write the failing tests**

Add to the bottom of `mod tests` in `crates/driver-core/src/lib.rs`, inside the closing brace:

```rust
    /// Distinguishable constants for the three framing methods, so a test can say
    /// which one landed and in what order. `profile`/`caps` diverge: framing reads
    /// neither, and a test that starts needing them is testing something else.
    struct FramingDriver;
    impl Driver for FramingDriver {
        fn profile(&self) -> &MachineProfile { unreachable!("framing does not read the profile") }
        fn caps(&self) -> MachineCaps { unreachable!("framing does not read the caps") }
        fn session_begin(&self) -> Vec<u8> { b"BEGIN".to_vec() }
        fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError> {
            Ok(format!("PASS{}", pass.polylines.len()).into_bytes())
        }
        fn pass_park(&self) -> Vec<u8> { b"PARK".to_vec() }
        fn session_end(&self) -> Vec<u8> { b"END".to_vec() }
        fn abort_bytes(&self) -> Option<Vec<u8>> { None }
    }

    #[test]
    fn only_the_first_pass_carries_the_session_prologue() {
        let job = Job { polylines: Vec::new(), settings: Settings::default() };
        assert_eq!(open_pass(&FramingDriver, &job, 0).unwrap(), b"BEGINPASS0".to_vec());
        assert_eq!(open_pass(&FramingDriver, &job, 1).unwrap(), b"PASS0".to_vec());
    }

    #[test]
    fn a_pass_parks_unless_it_is_the_last_one() {
        assert_eq!(close_pass(&FramingDriver, 0, 2), b"PARK".to_vec(), "another Pass follows, so park");
        assert_eq!(close_pass(&FramingDriver, 1, 2), b"END".to_vec(), "the last Pass closes the session");
        // The boundary a caller gets wrong: a one-Pass job's only Pass is also its last,
        // so it must close rather than park.
        assert_eq!(close_pass(&FramingDriver, 0, 1), b"END".to_vec());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p driver-core --locked pass`

Expected: FAIL to compile — `cannot find function 'open_pass' in this scope` and the same for `close_pass`.

- [ ] **Step 3: Write the implementation**

Add to `crates/driver-core/src/lib.rs` immediately after `write_all` (after the closing brace at `:76`, before `#[derive(Default)] pub struct MockTransport`):

```rust
/// The bytes that open Pass `index`: the session prologue on the first Pass, then
/// the encoded Pass itself.
///
/// `DeviceManager` writes these, waits for the machine, then writes `close_pass`.
/// The two together are one Pass on the wire, so a caller that wants the whole Pass
/// at once — `cuthulhu cut --dry-run` — concatenates them rather than restating when
/// a prologue is owed.
pub fn open_pass(d: &dyn Driver, job: &Job, index: usize) -> Result<Vec<u8>, DriverError> {
    let mut bytes = if index == 0 { d.session_begin() } else { Vec::new() };
    bytes.extend(d.encode_pass(job)?);
    Ok(bytes)
}

/// The bytes that close Pass `index` of `total`: park between Passes, end the
/// session after the last one.
pub fn close_pass(d: &dyn Driver, index: usize, total: usize) -> Vec<u8> {
    if index + 1 < total { d.pass_park() } else { d.session_end() }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p driver-core --locked`

Expected: PASS, including the two new tests and every existing `driver-core` test.

- [ ] **Step 5: Commit**

```bash
git add crates/driver-core/src/lib.rs
git commit -m "Give the framing of a Pass one home, since a comment was all that held the dry run to it"
```

---

### Task 2: The worker reads its framing from those functions

**Files:**
- Modify: `crates/driver-core/src/manager.rs:11-12` (imports), `:416-443` (`finish_pass`), `:447-478` (`run_from_pass`), `:480-490` (`pass_byte_len`)

**Interfaces:**
- Consumes: `open_pass`, `close_pass` from Task 1.
- Produces: nothing new. Every signature in `manager.rs` is unchanged; this task only changes where three Driver calls come from.

No new test. This task is behaviour-preserving, and the tests that prove it already exist —
`two_pass_job_frames_session_once_and_pauses_for_swap` counts one prologue and one epilogue on bytes a
`TeeTransport` actually received, and `plain_cut.rs`'s
`a_cancel_while_parked_for_confirmation_is_reported_as_a_cancel` pins the exact byte total
`pass_byte_len` recomputes for a Pass parked in `AwaitingCompletion`
(`cancel_mid_transmit_stops_writes_sends_abort_and_confirms_stop` covers a different path — its count
comes from `transmit_bytes`'s counter, not `pass_byte_len`). Writing a new failing test here would
mean inventing behaviour that must not change.

- [ ] **Step 1: Record the green baseline**

Run: `cargo test -p driver-core --locked`

Expected: PASS. Note the test count — the same tests must pass at Step 5.

- [ ] **Step 2: Add the imports**

In `crates/driver-core/src/manager.rs`, change the `use crate::{...}` line at `:12`:

```rust
use crate::{close_pass, open_pass, write_all, DeviceBackendFactory, DeviceInfo, Driver, Job, Transport, TransportError};
```

- [ ] **Step 3: Take `finish_pass`'s epilogue from `close_pass`**

Replace the body of `finish_pass` (`crates/driver-core/src/manager.rs:425-442`, everything between the
signature's `{` and its closing `}`) with:

```rust
    rep.emit(state, total_passes, job_id, DeviceEventKind::PassComplete(pass_index));
    write_all(transport, &close_pass(driver, pass_index, total_passes)).map_err(DeviceError::from)?;
    if pass_index + 1 < total_passes {
        let next_pass_index = pass_index + 1;
        emit_for(job_id, state, DeviceState::WaitingForColorSwap { job_id, next_pass_index }, total_passes, rep);
        Ok(PassRunOutcome::Paused { next_pass_index })
    } else {
        rep.emit(state, total_passes, job_id, DeviceEventKind::JobComplete);
        // Set after `JobComplete` goes out, not before: that event still carries the
        // mid-flight status, and a caller renders it — a "completed" outcome attached
        // to a `Sending` phase would read as a cut both finishing and still running.
        rep.set_ended(Some(Ended::Completed));
        // The job is over, so `Idle` reports no pass count. `total_passes` is a
        // parameter, not worker state, so nothing stale can outlive this call.
        emit_for(job_id, state, DeviceState::Idle, 0, rep);
        Ok(PassRunOutcome::Done)
    }
```

The write moves out of the branch; the branch now decides only `Paused` vs `Done`. Event order is
unchanged: `PassComplete`, then the epilogue write, then `JobComplete`, then `ended`. The two existing
comments are carried across verbatim — they document why `set_ended` follows `JobComplete` and why
`Idle` reports no pass count, and both facts still hold.

Leave the doc comment above the signature (`:414-415`) as it is.

- [ ] **Step 4: Take `run_from_pass`'s prologue and `pass_byte_len`'s recompute from `open_pass`**

In `run_from_pass`, replace `crates/driver-core/src/manager.rs:457-462`:

```rust
    let total_passes = passes.len();
    let bytes = open_pass(driver, &passes[pass_index].job, pass_index)
        .map_err(|e| DeviceError::Io(format!("{e:?}")))?;
```

Then replace `pass_byte_len` entirely (`:480-490`, doc comment included):

```rust
/// The byte length of an already-fully-transmitted Pass, from the same function
/// that produced those bytes — used by `Command::Cancel` to report
/// `submitted_bytes` for a job parked in `AwaitingCompletion`. Errors fall back
/// to 0: encoding already succeeded once to get here, and a cancel must not fail
/// because a byte count could not be recomputed.
fn pass_byte_len(driver: &(dyn Driver + Send), passes: &[CutPass], pass_index: usize) -> usize {
    open_pass(driver, &passes[pass_index].job, pass_index).map_or(0, |b| b.len())
}
```

- [ ] **Step 5: Run the tests to verify nothing moved**

Run: `cargo test --workspace --locked`

Expected: PASS, same tests as Step 1 plus the rest of the workspace. If
`two_pass_job_frames_session_once_and_pauses_for_swap` or
`cancel_mid_transmit_stops_writes_sends_abort_and_confirms_stop` fails, the refactor changed the wire
bytes — fix the code, not the test.

- [ ] **Step 6: Commit**

```bash
git add crates/driver-core/src/manager.rs
git commit -m "Read the worker's own framing from the functions that state it, not from three inline copies"
```

---

### Task 3: The dry run stops restating the rule

**Files:**
- Modify: `crates/cli/src/pipeline.rs:2` (import), `:27-40` (`pass_stream_bytes` → `dry_run_pass_bytes`)
- Modify: `crates/cli/src/main.rs:4` (import), `:120`, `:177` (call sites)
- Modify: `crates/cli/tests/dry_run.rs:2` (import), `:43`, `:57`, `:58` (call sites)

**Interfaces:**
- Consumes: `open_pass`, `close_pass` from Task 1.
- Produces: `pub fn dry_run_pass_bytes(d: &dyn Driver, job: &Job, i: usize, total: usize) -> Result<Vec<u8>, String>`, replacing `pass_stream_bytes` with the same signature and the same bytes.

- [ ] **Step 1: Replace the helper**

In `crates/cli/src/pipeline.rs`, change the import at `:2`:

```rust
use driver_core::{close_pass, open_pass, DeviceBackendFactory, Driver, Job, Settings};
```

Then replace `pass_stream_bytes` and its doc comment (`:27-40`) with:

```rust
/// The whole of Pass `i` of `total` on the wire, for `--dry-run`: what
/// `DeviceManager` writes before it waits for the machine, then what it writes
/// after. Both halves come from `driver-core`, so this cannot say something a cut
/// would not.
pub fn dry_run_pass_bytes(d: &dyn Driver, job: &Job, i: usize, total: usize) -> Result<Vec<u8>, String> {
    let mut bytes = open_pass(d, job, i).map_err(|e| format!("encode: {e:?}"))?;
    bytes.extend(close_pass(d, i, total));
    Ok(bytes)
}
```

The old comment promised the output was "framed exactly as `DeviceManager` transmits them"; that
promise is now structural, so the comment says where the bytes come from instead of asserting they
match.

- [ ] **Step 2: Follow the rename in `main.rs`**

`crates/cli/src/main.rs:4`:

```rust
use cli::pipeline::{check_color_flag_scope, check_interactive, dry_run_pass_bytes, plan_cut_from_svg, plan_plain_cut, Device};
```

`:120` (the plain path):

```rust
                    let bytes = dry_run_pass_bytes(driver.as_ref(), &plan.passes[0].job, 0, 1)?;
```

`:177` (inside `cut_by_color`):

```rust
            let bytes = dry_run_pass_bytes(d.as_ref(), &pass.job, i, passes.len())?;
```

- [ ] **Step 3: Follow the rename in the test**

`crates/cli/tests/dry_run.rs:2`:

```rust
use cli::pipeline::{dry_run_pass_bytes, plan_cut_from_svg, Device};
```

`:43`:

```rust
            String::from_utf8(dry_run_pass_bytes(d.as_ref(), &pass.job, i, passes.len()).unwrap()).unwrap()
```

`:57-58`:

```rust
    let c0 = dry_run_pass_bytes(cameo.as_ref(), &passes[0].job, 0, passes.len()).unwrap();
    let c1 = dry_run_pass_bytes(cameo.as_ref(), &passes[1].job, 1, passes.len()).unwrap();
```

Every assertion in that test stays exactly as it is. It asserted the right things; it was asserting
them about a copy.

- [ ] **Step 4: Run the workspace tests**

Run: `cargo test --workspace --locked`

Expected: PASS. `multi_pass_dry_run_parks_between_passes_like_the_device_manager` in particular —
`IN;` once, `PU;` closing both HPGL Passes, and `FN0` on the Cameo's last Pass only.

- [ ] **Step 5: Check the dry run by hand**

```bash
cargo run -p cli -- cut crates/cli/tests/fixtures/square.svg --device cameo5 --dry-run | head -20
```

Expected: a hex-and-ASCII dump, unchanged in shape from before this plan — 16 bytes per line, hex
left, printable ASCII right.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/pipeline.rs crates/cli/src/main.rs crates/cli/tests/dry_run.rs
git commit -m "Let the dry run ask driver-core how a Pass is framed, instead of answering for itself"
```

---

## Done when

- `cargo test --workspace --locked` passes.
- `git diff --stat main` shows no change to `Cargo.lock`, `apps/desktop/ui/dist/`, or any TypeScript file.
- `grep -rn "session_begin\|pass_park\|session_end" crates/ --include=*.rs` outside `driver-core/src/lib.rs`, the driver crates that implement them, and their tests returns nothing: no caller sequences those three by hand any more.
