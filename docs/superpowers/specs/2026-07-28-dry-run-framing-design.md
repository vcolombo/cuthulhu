<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Session framing in one place — design

Date: 2026-07-28
Status: approved (brainstorming complete)

## Purpose

Candidate 2 of `docs/superpowers/reviews/2026-07-27-architecture-review.md`: `pass_stream_bytes`
(`crates/cli/src/pipeline.rs:31-40`) reproduces a rule owned by `driver-core`'s worker —
`session_begin` before the first pass, `pass_park` between, `session_end` after the last. Its doc
comment is the contract, and a comment is all that holds it: change the worker's framing and nothing
fails, because `dry_run.rs` asserts against the copy.

Move the byte rules into `driver-core` so the worker and `--dry-run` read them from one place.

## Divergence from the review

The review proposes making `--dry-run` a fourth `Transport` and driving it through the real
`DeviceManager`. This design does not, and the reasons are worth recording because the alternative
looks strictly better until you price it.

**A recording transport cannot preview the session it claims to preview.** The worker writes
`driver.status_query()` through the same `Transport::write` as the geometry, and
`resolve_pass_completion` (`manager.rs:339-374`) polls at 250 ms until the device answers ready. On
hardware, a pass that takes forty seconds emits about 160 ENQs. A dry-run transport has to answer
ready on the first poll or burn the 60 s deadline, so the dump shows exactly one. The poll count is
an artifact of the fake, not a preview of anything.

**It makes deterministic output non-deterministic.** Today `--dry-run` is a pure function of SVG,
device and settings. Routing it through the worker makes it a function of thread scheduling too. The
consumer most likely to care is an agent diffing or hashing the output, which is precisely the
consumer the transport approach was meant to serve better.

**The completion policy has to be answered, not just recorded.** A Puma
(`needs_operator_pass_confirm`) parks in `AwaitingCompletion`, and a multi-pass job parks again for
each colour swap, so a dry run would need an operator that answers pauses instantly and silently —
a third `Operator` variant, exempt from `run`'s deliberate refusal to answer pauses unattended
(`cut.rs:123-127`). That refusal exists because answering a pause with nobody present starts the next
pass into a machine that may still be moving. Adding an exemption to a safety rule so that a preview
can print is the wrong shape of change.

What the review's approach would genuinely deliver and this one does not: `dry_run.rs` exercising the
real worker loop. That coverage already exists elsewhere —
`manager.rs::two_pass_job_frames_session_once_and_pauses_for_swap` asserts one prologue and one
epilogue on the bytes a `TeeTransport` actually received — and after this change it asserts them
about the same functions the dry run calls. Drift is closed either way.

## Scope

In scope: two new functions in `driver-core`, three callers changed to use them, one duplicated
assembly of the prologue-plus-Pass sequence deleted from the worker (the encode itself stays;
`pass_byte_len` still calls `encode_pass` via `open_pass`).

Out of scope, decided deliberately:

- **`--dry-run --json`.** Hex-and-ASCII is a human artifact, and this project is meant to be usable
  by agents too. This design leaves that easy — the bytes for a pass become one call — and does not
  build it. No consumer has asked yet.
- **The plain/`--by-color` split in `main.rs`.** `Command::Cut` branches to a separate dry-run print
  for the plain path (`main.rs:118-123`) and another inside `cut_by_color` (`:173-181`). Merging them
  is Candidate 6's plan-assembly question, not this one.
- **`Device` and `resolve_device_info`.** Candidate 3.

## Decisions

**The seam is where the worker already breaks the pass: at the completion poll.** The worker does not
emit a pass's framing contiguously. `run_from_pass` writes `session_begin` + payload, *then* polls,
*then* `finish_pass` writes park-or-end. A single `frame_pass` function could not be shared with it
without inverting that order, so the rule splits into the same two halves the worker writes:

```rust
/// The bytes that open Pass `index`: the session prologue on the first Pass, then
/// the encoded Pass itself. `DeviceManager` writes these, waits for the machine,
/// then writes `close_pass` — so the two together are one Pass on the wire.
pub fn open_pass(d: &dyn Driver, job: &Job, index: usize) -> Result<Vec<u8>, DriverError>

/// The bytes that close Pass `index` of `total`: park between Passes, end the
/// session after the last one.
pub fn close_pass(d: &dyn Driver, index: usize, total: usize) -> Vec<u8>
```

`open`/`close` rather than prologue/epilogue: `CONTEXT.md:102` already defines a **Driver** as what
produces the bytes "to open, park and close a cutting session".

**They live in `crates/driver-core/src/lib.rs`, beside `write_all`.** Same kind of thing — a free
function over the traits the crate defines — and it needs no new module for twenty lines.

**They take `&dyn Driver`, and the worker's `&(dyn Driver + Send)` coerces.** Proven in-tree:
`probe_is_cutter(t.as_mut(), d.as_ref(), &info)` (`manager.rs:606`) already passes a
`Box<dyn Driver + Send>` to a `&dyn Driver` parameter.

**`pass_stream_bytes` is renamed, not just re-implemented.** It becomes
`dry_run_pass_bytes`, and its doc comment stops promising fidelity — the name says what it is for,
and fidelity is now structural rather than asserted. It stays in `cli::pipeline` because two
commands call it and concatenating the two halves is not `driver-core`'s business.

### Rejected alternatives

**Delete the CLI helper and inline the two calls at both sites.** Four lines saved in `pipeline.rs`,
paid for by the same concatenation appearing twice in `main.rs`. The helper is not a restatement of
the rule any more; it is one line of `extend`.

**Give `Driver` a provided method (`fn framed_pass(&self, ...)`).** Every driver would inherit it,
including drivers for which the sequencing is wrong, and it would sit in the trait `driver-core`
deliberately keeps to what a driver must implement.

## Architecture

```
                      driver-core::open_pass / close_pass
                                    ▲               ▲
              ┌─────────────────────┘               └─────────────────────┐
              │                                                           │
   DeviceManager worker                                        cli::dry_run_pass_bytes
   run_from_pass ─ open_pass                                       open_pass + close_pass
   (transmit, poll the machine)                                          │
   finish_pass  ─ close_pass                                             ▼
              │                                                    print_hex_ascii
              ▼
          Transport
```

The three Driver calls that state the rule — `session_begin`, `pass_park`, `session_end` — are
reachable from exactly one place each after this change.

## Components

| File | Change |
| --- | --- |
| `crates/driver-core/src/lib.rs` | Add `open_pass` and `close_pass` beside `write_all` |
| `crates/driver-core/src/manager.rs:458-462` | `run_from_pass` takes its prologue + payload from `open_pass` |
| `crates/driver-core/src/manager.rs:416-443` | `finish_pass` writes `close_pass(...)` once, then keeps only the park-vs-finish *state* branch |
| `crates/driver-core/src/manager.rs:484-490` | `pass_byte_len` collapses to `open_pass(...).map_or(0, \|b\| b.len())` — a third copy of "prologue on the first pass" |
| `crates/cli/src/pipeline.rs:27-40` | `pass_stream_bytes` → `dry_run_pass_bytes`, four lines over the two new functions |
| `crates/cli/src/main.rs:4,120,177` | Follow the rename |
| `crates/cli/tests/dry_run.rs:2,43,57-58` | Follow the rename |

`finish_pass` keeps its structure. Today it branches once and does two things in each arm (write the
bytes, move the state); after, the write is unconditional and the branch decides only `Paused` vs
`Done`. Event order is unchanged: `PassComplete` before the write, `JobComplete` after it, `ended`
set after `JobComplete` goes out — the reason for that last one is documented at `manager.rs:436-438`
and this change must not disturb it.

## Error handling

Unchanged. `open_pass` returns `DriverError` because `encode_pass` does; the worker keeps mapping it
to `DeviceError::Io(format!("{e:?}"))` and the CLI keeps mapping it to `format!("encode: {e:?}")`.
`close_pass` returns bytes and cannot fail — the three Driver methods it calls all return `Vec<u8>`
infallibly.

`pass_byte_len` keeps swallowing an encode failure as 0, and keeps the comment saying why: it runs
during a cancel, on a pass that already encoded successfully once, and a cancel must not fail
because a byte count could not be recomputed.

## Testing

**New, in `crates/driver-core/src/lib.rs`'s test module** (which has no `Driver` fake today, so one
arrives with them — a dozen lines returning distinguishable constants for the three framing methods):

1. `open_pass` prefixes the session prologue on Pass 0 and on no other Pass.
2. `close_pass` parks between Passes and ends the session after the last one — asserted at the
   boundary, `close_pass(d, 0, 2)` vs `close_pass(d, 1, 2)`, and on the single-Pass case
   `close_pass(d, 0, 1)`, which must end rather than park.

These are the twelve-numbers-in-one-table equivalent for this candidate: the rule becomes data a test
can assert, where today it is asserted only through two callers that could agree with each other and
both be wrong.

**Existing tests that must stay green untouched**, and what each now covers:

- `manager.rs::two_pass_job_frames_session_once_and_pauses_for_swap` — one prologue and one epilogue
  on the wire, from a real two-pass job. This is the test that pins the worker to the shared
  functions; it needs no edit, which is the point.
- `dry_run.rs::multi_pass_dry_run_parks_between_passes_like_the_device_manager` — keeps its
  assertions and gains its title back: it now compares against the functions the manager uses rather
  than a copy of them. Rename-only edit.
- `dry_run.rs::plain_dry_run_refuses_geometry_off_the_bed` — untouched; preflight is not in scope.
- `plain_cut.rs` — untouched. It drives `cut::run` against a fake device and asserts `BEGIN` / `PASS`
  / `PARK` / `END` on the recorded bytes, so it covers the worker's use of the new functions
  end to end without knowing they exist.
- `manager.rs::cancel_mid_transmit_stops_writes_sends_abort_and_confirms_stop` — asserts a reported
  byte total against payload bytes that actually landed, but that count comes from
  `transmit_bytes`'s own counter; it never reaches `pass_byte_len`, which only runs from
  `Command::Cancel` on a Pass parked in `AwaitingCompletion`. `plain_cut.rs`'s
  `a_cancel_while_parked_for_confirmation_is_reported_as_a_cancel` is the one test that reaches it,
  and pins the exact byte count `pass_byte_len` recomputes.

No `MANUAL-CHECKLIST.md` entry: the bytes on the wire are unchanged by construction, and the tests
above are what say so.

## Verification

```sh
cargo test --workspace --locked
```

No UI, no dependency, no `dist/` rebuild — this change is confined to two Rust crates.
