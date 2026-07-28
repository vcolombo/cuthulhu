<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Trace's input contract — design

Date: 2026-07-27
Status: approved (brainstorming complete)

## Purpose

Candidate 1 of `docs/superpowers/reviews/2026-07-27-architecture-review.md`, and its top
recommendation. `trace` is a deep module behind a shallow interface: its public surface is vtracer's
parameter list with `pub` fields, so every caller restates what the module knows. Twelve range
numbers across three languages, five defaults stated three times, a 256 MiB input cap implemented
twice in full, and a sentinel error matched by its `Display` string.

Give `trace` an interface in the units its users think in, and let the vtracer vocabulary stop at the
module boundary.

## One correction to the review

The review presents the inverted `detail` control as an accident — *"not a refactor smell; it is a
bug with an architectural cause"* — and reads the two opposite hints as equally arbitrary. Half of
that is wrong, and the half that is wrong changes the fix.

The inversion is a **recorded decision**. `docs/superpowers/specs/2026-07-25-trace-design.md:111`
says the Detail slider *"displays an inverted scale of `length_threshold` (slider up = more detail =
lower threshold), so the empty-state hint 'raise detail' is directionally correct."* That is the
right call: a control named Detail should go up when detail goes up.

The same spec, at line 133, specifies the CLI's empty-result message as `"Nothing traced — adjust
--speckle/--detail"` — deliberately direction-neutral, because the CLI carried the raw threshold and
no direction word could be right for both interfaces. The implementation drifted from that into
`"nothing traced — lower --speckle or lower --detail"` (`crates/cli/src/main.rs:192`), which is
correct for the CLI's units and the exact opposite of the dialog's advice.

So the CLI is the side out of step, not the dialog. The fix is not to remove the inversion but to
**move it into `trace` and have both callers speak the user-facing scale.** That is also what makes
a single shared message possible, which is the point.

## Scope

In scope:

- `trace` exports `TraceControls` (user units) and a `CONTROLS` spec table; `TraceOptions` stops
  being public.
- `trace::read_image` owns the 256 MiB input ceiling.
- `TraceError` gains `Input` and `code()`; `EmptyResult` gains a real operator message.
- A `trace_controls` IPC command; `trace_image` moves to `TraceControls` and `IpcError`.
- `--detail` flips to user units. `toOptionsDto` and its `13.5` are deleted.
- A `CHANGELOG.md`, created to record the flip.

Out of scope, decided deliberately:

- **`preview_png`'s base64 data URL** (`2026-07-25-trace-design.md:87`). A recorded decision with
  its own reasoning, untouched by the input contract.
- **Clap-native range enforcement.** See Decisions.
- **Whether 3.5–10 is a good user-facing scale at all.** A UX question. This change makes it one
  scale instead of two; it does not relitigate the numbers.
- **`MAX_DIM` as a user control.** It stays a constant. It stops being hardcoded in a UI string.
- **The trace path's own IPC width.** Candidate 7's territory.

## Decisions

**`--detail` flips, and the flip is not hidden.** `cuthulhu trace --detail 4.0` today means vtracer's
raw threshold — 4.0 is *very detailed*. Afterwards `--detail 4.0` means what the dialog's Detail 4.0
means: *very coarse*. Same command, silently different geometry. Accepted because nothing has been
released (no git tags; every crate is at `0.1.0`), and because the alternative — two interfaces using
one word for opposite things — is the defect being fixed. The **default is unchanged in effect**:
`4.0` raw becomes `9.5` user-facing, the same trace.

**Direction lives in the table, not in a magic number.** `viewmodel.ts:16` converts with `13.5 - c.detail`,
which is correct only because `3.5 + 10.0 = 13.5` — the range happens to be its own reflection. Inside
`trace` the conversion becomes `DETAIL.min + DETAIL.max - detail`, derived from the same table entry
that states the bounds, so moving a bound cannot silently break the reflection.

**TypeScript holds no conversion at all.** `toOptionsDto` is deleted rather than rewritten: the
webview sends user units and `trace` inverts. This is the test that the seam landed in the right
place — a conversion function at a boundary is that boundary admitting it is in the wrong units.

**One enforcement point, not clap's.** `clap::value_parser!` has no range form for `f64`, so
range-checking in clap means a custom `ValueParser` per control — more code than `trace::validate`
naming the range for all four, and a second place to state the bounds. The review's complaint that
"clap does not enforce the ranges" is answered by making *one* enforcement point read from the table,
not by moving enforcement into clap. `--speckle 200` still fails at run time rather than at parse
time, with `filter_speckle must be 0–16`.

**`help` restates its own range, and a test pins it.** clap's derive needs a literal for `help`, and
dropping `(0–16)` from `--help` is a real regression. So each `ControlSpec` carries a `help` string
that names its bounds in prose, *adjacent to the numbers it restates*, with a unit test asserting
each `help` contains its own `min` and `max`. Duplication one struct literal away that a test
catches is not the same defect as duplication in a second language that nothing checks.

**`code()` is a method, not a serde tag.** The CLI wants `Display` and nothing else. Only the desktop
pays for the IPC shape.

**Authorization does not move.** `pick_image`, `AuthorizedImages`, `canonical`, and `authorized_path`
stay in `apps/desktop/src/ipc.rs`. `authorized_path` returns the *resolved* path precisely so the
caller opens exactly what was authorized, with no window between the check and the open
(`ipc.rs:194-199`). `trace::read_image(&real)` preserves that: it opens the canonical path, which is
what `read_image_file(&real)` does today. A later change that lets the desktop pass the user's
original path and re-resolve inside `trace` would reopen that window.

## Shape

### The control table

```rust
/// One user-facing trace control: what it is called, what it accepts, where it starts.
pub struct ControlSpec {
    pub name: &'static str,    // "speckle" | "smoothing" | "detail" | "colors"
    pub label: &'static str,   // the dialog's slider label
    pub help: &'static str,    // clap's help; states its own range in prose
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
    pub color_only: bool,      // inert in binary mode
}

pub const CONTROLS: [ControlSpec; 4] = [ /* speckle, smoothing, detail, colors */ ];
```

Values preserved exactly as they are today, with `detail` stated in user units:

| name | label | min | max | step | default | color_only |
| --- | --- | --- | --- | --- | --- | --- |
| `speckle` | Ignore speckles | 0 | 16 | 1 | 4 | no |
| `smoothing` | Smoothing | 0 | 180 | 1 | 60 | no |
| `detail` | Detail | 3.5 | 10 | 0.5 | 9.5 | no |
| `colors` | Colors | 1 | 8 | 1 | 6 | yes |

`validate` generates its messages from the table, clap takes `default_value_t` from it, and the
dialog renders its sliders from it.

`mode` is deliberately **not** in the table. It is a two-value radio, not a bounded numeric slider,
and it has no range, step, or `color_only` to state — an entry for it would be four fields of
`None`. It travels in the payload beside the table instead:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceControlSpecs {
    pub controls: Vec<ControlSpec>,   // the four sliders, in display order
    pub default_mode: TraceMode,      // Binary
    pub max_dim: u32,                 // MAX_DIM
}
```

This is what `trace_controls` returns, and it is the whole of what TypeScript knows about trace
inputs.

### The boundary

```
              before                                    after
  TS TraceControls ──13.5-x──► TraceOptions      TS TraceControls ─────► trace::TraceControls
  clap --detail (raw) ───────► TraceOptions      clap --detail (user) ─► trace::TraceControls
                                    │                                          │
                                    ▼                                (min + max - detail, private)
                              vtracer::Config                            vtracer::Config
```

```rust
pub struct TraceControls {
    pub mode: TraceMode,
    pub speckle: u8,     // 0–16
    pub smoothing: u8,   // 0–180
    pub detail: f64,     // 3.5–10, higher = more detail
    pub colors: u8,      // 1–8
}

pub fn trace(image_bytes: &[u8], controls: &TraceControls) -> Result<TraceResult, TraceError>;
```

This **replaces** `trace`'s current signature; there is no second entry point taking `TraceOptions`,
which would be the shallow wrapper this change exists to avoid. `TraceOptions` becomes a private
conversion step. Nothing outside `trace` names a vtracer parameter.

### Input

```rust
/// Ceiling on the source file, since `MAX_DECODE_ALLOC` only applies once the bytes are resident.
pub const MAX_INPUT_FILE_BYTES: u64 = 256 * 1024 * 1024;

pub fn read_image(path: &Path) -> Result<Vec<u8>, TraceError>;
fn read_image_capped(path: &Path, cap: u64) -> Result<Vec<u8>, TraceError>;
```

`cap` is a parameter on the private form so the bound can be exercised with a handful of bytes rather
than a quarter gigabyte — the trick `ipc.rs:231-234` already uses, kept.

Both properties the current copies exist for are preserved, and the comments explaining *why* move
with the code: everything happens through **one open handle** (`File::metadata` is `fstat` on the
handle, so it cannot describe a different file than the one about to be read, unlike stat-then-read
on a pathname), and `take(cap + 1)` is what actually bounds the read.

`load_image_preview` reads through the same function, so the thumbnail path stops carrying its own
copy of the ceiling.

### Errors

```rust
pub enum TraceError { Input(String), InvalidOption(String), Decode(String), Trace(String), EmptyResult }

impl TraceError {
    /// Stable identifier for a caller that must branch on the kind of failure rather than
    /// show its text.
    pub fn code(&self) -> &'static str;  // "input" | "invalid_option" | "decode" | "trace" | "empty"
}
```

`Input` is new. A 300 MiB file is not a decode failure; folding it into `Decode` was only tenable
while nothing branched on the difference.

`EmptyResult`'s `Display` stops being the bare `"empty"` and becomes:

> `nothing traced — lower the speckle filter or raise detail`

Both interfaces print it verbatim. The CLI's `map_err` (`main.rs:190-194`) is deleted. The dialog
keeps its distinct empty-state rendering but branches on `code === "empty"` rather than
`message === "empty"`. The message is correct in both places only because both now measure detail the
same way.

`trace_image` returns `IpcError { code, message }` like every other command. `ipcErrorCode()` already
exists in `ui/src/ipc.ts` and is unused by this path today.

## Call sites

| File | Change |
| --- | --- |
| `crates/trace/src/lib.rs` | `+ControlSpec`/`CONTROLS`, `+TraceControls`, `+read_image`, `+TraceError::Input`/`code()`; `TraceOptions` → private; `EmptyResult` message |
| `crates/cli/src/main.rs` | delete `read_image_capped` and `MAX_INPUT_FILE_BYTES` (~35 lines); clap defaults and help from `CONTROLS`; `--detail` in user units; delete the `EmptyResult` `map_err`; report `downscaled` |
| `apps/desktop/src/ipc.rs` | delete `read_capped`, `too_large`, `read_image_file`, `MAX_INPUT_FILE_BYTES` (~50 lines); `trace_image` takes `TraceControls`, returns `IpcError`; `+trace_controls` |
| `apps/desktop/ui/src/ipc.ts` | `TraceOptionsDto` → `TraceControlsDto`; `+ControlSpec`, `+traceControls()` |
| `apps/desktop/ui/src/trace/viewmodel.ts` | delete `toOptionsDto` and `defaultControls`; `acceptError` branches on code |
| `apps/desktop/ui/src/trace/TraceDialog.tsx` | sliders from specs; `2048` from the payload; `color_only` from the spec |
| `apps/desktop/ui/e2e/smoke.spec.ts` | stub `trace_controls` |
| `CONTEXT.md` | define **TraceControls** |
| `CHANGELOG.md` | new; records the `--detail` flip |

`trace_controls` returns the table together with `MAX_DIM`, which retires the hardcoded `2048` in
`TraceDialog.tsx:132`. The dialog gates its sliders and its trace effect on the specs having arrived,
the way `CutDialog` gates on caps.

The CLI's silent downscale is fixed in passing: `cuthulhu trace` on a 6000 px image reduces to 2048
and says nothing today, though `TraceResult::downscaled` has carried the fact since the crate was
written. One line beside the existing path count.

## CHANGELOG

`CHANGELOG.md` at the repository root, [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
format, starting at `## [Unreleased]`. No release history is back-filled: there are no git tags and
every crate is at `0.1.0`, so any prior "release" would be invented. Its first entry is the `--detail`
flip under `### Changed`.

The flip is also stated in the flag's own `help` text (`higher = more detail`), which is where a CLI
user actually meets it.

## Tests

- **`validate` rejects each control one step outside its own bounds**, read from `CONTROLS` rather
  than typed into the test. The ranges become data a test asserts; today the UI's copy is checked
  only by `viewmodel.test.ts:14` asserting the magic `13.5`, and clap's copy is checked by nothing.
- **Each spec's `help` contains its own `min` and `max`** — pins the one adjacent restatement.
- **The detail inversion round-trips**: `detail == DETAIL.max` yields `length_threshold == DETAIL.min`,
  and the reverse. This is the regression test for the flip, and it fails if either bound moves
  without the other.
- **`read_image_capped` refuses one byte over a small cap and accepts exactly at it.**
- **`TraceError::code()` covers every variant** (exhaustive `match`, so a new variant will not
  compile without one).
- **TS**: the `13.5` assertion at `viewmodel.test.ts:14` is **deleted, not ported** — that test
  existed to pin a conversion, and the conversion is gone. Porting it would test the same arithmetic
  in a new home; the inversion round-trip covers the actual invariant.
- **TS**: `acceptError` maps code `"empty"` to the empty state and anything else to `error`.
- Existing viewmodel tests for debounce and staleness are untouched.

## Implementation order

Three commits, split by what can stay green rather than by layer.

1. **`trace::read_image` and `TraceError::Input`, and both callers delete their copies.** Additive to
   `trace`'s surface; no signature changes, no behaviour change, no wire change. `Input` lands here
   because `read_image` is what returns it. Workspace green, app unaffected.
2. **The control table, `TraceControls`, `code()`, the `EmptyResult` message — all of Rust at
   once**, plus the `--detail` flip, `trace_controls`, and `CHANGELOG.md`. `trace()`'s signature
   changes, so `trace`, `crates/cli`, and `apps/desktop` cannot move independently.
3. **UI**: specs over IPC, `toOptionsDto` deleted, e2e stub, `dist/` rebuilt.

Commit 2 leaves the desktop's trace dialog broken at run time — it still posts
`{filterSpeckle, …}` at a command now expecting `{speckle, …}` — until commit 3 lands. That gap is
internal to the branch, which merges as a unit, but it is worth knowing before bisecting into it.
Nothing catches it automatically: the e2e suite drives an in-page fake, not the real command.

Backend before UI is also forced mechanically: `ui/dist` is committed and CI rebuilds it, so any
commit touching `ui/src` must carry a rebuilt bundle. Landing `trace_controls` in commit 2 keeps the
UI work to a single bundle rebuild.
