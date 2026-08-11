<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Cut Host correctness fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the seven defects an independent review found across phase 2, three of which can cut the wrong thing, cut it twice, or say a blade has stopped when it has not.

**Architecture:** Fixes span `crates/cut-host`, `crates/driver-core`, `apps/desktop/src` and `apps/desktop/ui`. Each task is independent; they are ordered so the two that touch the same file do not collide.

**Source:** `.superpowers/sdd/codex-review-findings.md` — the full review, verbatim. Read the finding before its task.

## Global Constraints

- `cargo test --workspace --locked` is what CI runs; `--locked` is mandatory.
- `apps/desktop/ui/dist/` is committed and CI fails on a stale bundle. Any `ui/src` change means `npm --prefix apps/desktop/ui run build` and committing `dist/` in the same commit. `npm test` does **not** typecheck (esbuild strips types) — only `run build` does.
- **A caller is told about a cut through one value.** Render from `CutStatus::actions`; never re-derive legality from a phase. `DeviceState` stays `pub(crate)`.
- `CONTEXT.md` is normative vocabulary: Pass, Job, Driver, Transport, Preflight, Cut Host. Never "proxy"/"server"/"relay"/"bridge".
- Comments explain why, not what. `// ponytail:` marks a deliberate simplification with its ceiling and upgrade path.
- **Several of these defects are guarded by comments asserting the bug away.** When you fix one, fix its comment — a comment that argues for the broken behaviour is worse than none, because the next reader trusts it.
- Commit subjects are imperative with the reason attached; keep the repo's `Co-Authored-By:` trailer.

---

### Task 1: refuse a dispatch whose aim moved after Preflight

**Finding:** review §1, first item. `ipc.rs:135` prepares Passes under the document lock, drops it, then calls `execute_cut`, which re-reads the current device at `device.rs:578`. The UI leaves other Connect buttons live while `cut()` is pending.

**Failure:** start a cut aimed at A; connect B while planning runs. The Passes approved for A are sent to B. Remote Preflight cannot catch it, because `execute_cut` supplies **B's own** `machine_id` — so the machine-mismatch check compares B against B.

**Files:** Modify `apps/desktop/src/device.rs`, `apps/desktop/src/ipc.rs`

- [ ] **Step 1:** Make `prepare_cut` return the `DeviceInfo` it planned against, alongside the Passes.
- [ ] **Step 2:** `execute_cut` takes that device and refuses if the current aim is not the same cutter — same `instance_id` **and** same `host`. Ids repeat across hosts (see Task 6), so `instance_id` alone is not identity.
- [ ] **Step 3:** Test: plan against A, change `connected` to B, dispatch, assert refusal and that nothing reached B.
- [ ] **Step 4:** `cargo test --workspace --locked`; commit.

---

### Task 2: give the authenticated request loop a header deadline

**Finding:** review §2. `serve.rs:261` passes `None` as the header deadline; `frame.rs:122` starts the body deadline only once all four length bytes have arrived, so `fill` retries forever on a partial header.

**Failure:** an authenticated client sends one byte and stops. Its worker never returns, so its `MAX_CLIENTS` slot is never released. Eight such connections and the daemon refuses every new one until restarted. **`SO_KEEPALIVE` does not help** — the peer is alive and acknowledging.

The `None` is correct for the gap *between* frames: a desktop that polls is idle then, and must not be dropped for it. It is wrong once a frame has begun. That distinction is already written in the comment above it and is simply not enforced.

**Files:** Modify `crates/cut-host/src/frame.rs`, `crates/cut-host/src/serve.rs`

- [ ] **Step 1:** In `read_frame`, once the **first** header byte has arrived, apply a deadline to the rest of the header — the frame is now owed. Idle-before-any-byte keeps waiting forever.
- [ ] **Step 2:** Test: send one byte, assert the read fails within the deadline rather than hanging.
- [ ] **Step 3:** Test: a client idle between whole frames is still not dropped — this is the property the `None` protects, and it must survive.
- [ ] **Step 4:** Correct the comment at `serve.rs:259-261` to say what is now true.
- [ ] **Step 5:** `cargo test --workspace --locked`; commit.

---

### Task 3: reuse a dispatch id when retrying, so a dropped reply cannot cut twice

**Finding:** review §3, first item. `device.rs:590` mints a fresh timestamp-based id per call, and its comment asserts "a fresh id per attempt: this is a new Job, not a retry of a dropped reply."

**Failure:** the daemon accepts dispatch `d1` and starts cutting; Wi-Fi drops before the reply. The operator sees a failure, waits, retries. The retry carries `d2`, so the daemon's dedupe cannot recognise it. **The material is cut twice.**

The dedupe exists for exactly this case and nothing on the desktop ever reuses an id, so today it can never fire. The comment argues the problem out of existence instead of solving it.

**Files:** Modify `apps/desktop/src/device.rs`, `apps/desktop/ui/src/cut/CutDialog.tsx`

- [ ] **Step 1:** Derive the id from the Job being dispatched, not the clock — the aimed device plus a hash of the Passes and settings, so the same Job retried yields the same id and a genuinely new Job does not.
- [ ] **Step 2:** Test: dispatching the same Job twice against a fixture that drops the first reply results in **one** Job on the host.
- [ ] **Step 3:** Test: two genuinely different Jobs to one cutter get different ids.
- [ ] **Step 4:** Replace the comment with the reasoning that now applies.
- [ ] **Step 5:** `cargo test --workspace --locked`; rebuild and commit `dist/` if `ui/src` changed.

---

### Task 4: make dedupe and admission one transition

**Finding:** review §3, second item. `host.rs:169` inserts the id, releases the lock, checks `actions.cut`, and spawns the worker later at `:183`.

**Failures, both real:**
- Busy cutter: A inserts `d1`; concurrent retry B sees it present and returns `Accepted`; A then observes Busy, removes `d1`, returns `Refused`. **B was told a Job was accepted that does not exist**, and the id is wrongly free again.
- Idle cutter: two distinct ids both observe `actions.cut` before either worker starts. Both get `Accepted`; one may hit Busy in its detached thread where only a log sees it — or run after the first finishes, accidentally implementing the queueing this design explicitly refuses.

An earlier fix made the *insert* atomic. The compound transaction — claim the id and claim the cutter, or neither — still is not.

**Files:** Modify `crates/cut-host/src/host.rs`

- [ ] **Step 1:** Hold one lock across claiming the id and admitting the Job, so no other request can observe a half-claimed state.
- [ ] **Step 2:** Test: two concurrent dispatches with the **same** id to a busy cutter — exactly one answer, and no id left dangling.
- [ ] **Step 3:** Test: two concurrent dispatches with **different** ids to an idle cutter — one `Accepted`, one `Refused` with Busy. Never two accepted, never a queued second Job.
- [ ] **Step 4:** `cargo test --workspace --locked`; commit.

---

### Task 5: stop offering a cut when cancellation could not confirm the machine stopped

**Finding:** review §5, first item. `manager.rs:507` computes `completion_known` from an ENQ poll; `status.rs:135` ignores it and publishes `actions.cut = true` for every cancellation, and `manager.rs:624` accepts `Command::Cut` from any `Cancelled` state.

`manager.rs:1380` states the position being overturned: *"`completion_known` has no place in `CutStatus` — nothing a caller can do."* There is something a caller can do: not start another Job.

**Failure:** cancel a Puma, which cannot confirm readiness at all, or a Cameo still busy past the short poll. The UI reports Ready and permits a new Job while the machine may still be executing buffered motion.

**Files:** Modify `crates/driver-core/src/status.rs`, `crates/driver-core/src/manager.rs`

- [ ] **Step 1:** `status_of` reads `completion_known`. When a cancel could not confirm the machine stopped, `actions.cut` is false and the status says why.
- [ ] **Step 2:** `Command::Cut` refuses from a `Cancelled` state whose completion was never confirmed. The operator reconnects — or confirms — rather than the software guessing.
- [ ] **Step 3:** Test both, asserting on `actions`, never on a phase.
- [ ] **Step 4:** Replace the comment at `manager.rs:1380`.
- [ ] **Step 5:** `cargo test --workspace --locked`; commit.

---

### Task 6: refuse to forget a host we cannot reach, with an explicit force

**Finding:** review §5, second item. `device.rs:284` blocks the forget only when the snapshot **succeeds** and reports active; every network error falls through to credential deletion at `:293`.

**Failure:** drop Wi-Fi during a Job and click Forget. The Pi keeps cutting; this desktop discards the token and the route it needs to cancel, resume or confirm when connectivity returns.

This reverses an earlier ruling. The old reasoning — a Pi that is off must stay forgettable — is still true, which is why the escape hatch is explicit rather than absent.

**Files:** Modify `apps/desktop/src/device.rs`, `apps/desktop/src/ipc.rs`, `apps/desktop/ui/src/hosts/deviceList.ts`, `apps/desktop/ui/src/cut/CutDialog.tsx`

- [ ] **Step 1:** `forget` refuses whenever it cannot confirm the host is idle — unreachable included — with a distinct error code from the busy case, since the operator's next move differs.
- [ ] **Step 2:** `forget_host` gains a `force` parameter. Forced, it discards regardless.
- [ ] **Step 3:** The UI offers force only after an unforced attempt failed, and the confirmation says plainly that a cut may still be running and this desktop will not be able to stop it.
- [ ] **Step 4:** Tests: unreachable refuses unforced, succeeds forced; active-and-reachable refuses unforced.
- [ ] **Step 5:** `cargo test --workspace --locked`, `npm --prefix apps/desktop/ui test`, `run e2e`, `run build`; commit `dist/`.

---

### Task 7: wire the daemon's active-cut shutdown guard

**Finding:** review §Other. `host.rs:199` `is_any_cut_active` has no caller outside tests, and `bin/cuthulhu-cutd.rs:34` goes straight into `serve` with no signal handling.

**Failure:** `systemctl stop`, a restart, or SIGTERM during a Job kills the process and abandons the cut. The spec says the daemon refuses to exit while any device reports active, mirroring the desktop's window-close guard, except on explicit force.

**Files:** Modify `crates/cut-host/src/bin/cuthulhu-cutd.rs`, `docs/cut-host.md`

- [ ] **Step 1:** Install a SIGTERM/SIGINT handler that refuses to exit while `is_any_cut_active()`, logging what is still running and where to see it.
- [ ] **Step 2:** A second signal, or a documented force, exits anyway — an operator who means it must not be trapped.
- [ ] **Step 3:** `docs/cut-host.md` gains the systemd implication: `TimeoutStopSec` must exceed a realistic cut, or systemd will `SIGKILL` past this guard and undo it.
- [ ] **Step 4:** Test the predicate wiring headlessly against `MockTransport`.
- [ ] **Step 5:** `cargo test --workspace --locked`; commit.

---

## Out of scope — filed as issues

- Status matched by `instance_id` without `host` (`CutDialog.tsx:345`); ids legitimately repeat across hosts via `usb:at:`/`serial:at:` fallbacks.
- The 2s status budget excludes time waiting for the per-host lock, which `list_devices` can hold for 30s.
- "Quit anyway" performs remote cancellation synchronously on the Tauri main thread (`main.rs:18`).
