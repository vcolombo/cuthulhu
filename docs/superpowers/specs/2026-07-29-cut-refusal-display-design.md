<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Every refusal says what happened, once — design

Date: 2026-07-29
Status: approved (brainstorming complete)

## Purpose

Candidate 6 of `docs/superpowers/reviews/2026-07-27-architecture-review.md`. `plan_cut` is the
chokepoint every cut goes through, and it refuses in ten distinct ways — three `CutError` variants,
one of which wraps seven `PreflightError` variants. Neither type implements `Display`, so each of
the two callers writes its own English for all ten:

- `apps/desktop/src/device.rs:198-231` — `map_cut_error` plus `map_preflight_error`, ten arms,
  each producing an IPC code and a message.
- `crates/cli/src/pipeline.rs:177-190` — `describe_cut_error`, five arms and a catch-all
  `format!("preflight: {e:?}")`.

The catch-all is where it has already cost the operator something. Four refusals —
`NonFiniteGeometry`, `DegeneratePolyline`, `MachineMismatch`, `OutputTooLarge` — reach a CLI user as
`Debug` output. A document built for a Puma, sent to a Cameo, prints:

```
preflight: MachineMismatch { document: "puma", device: "cameo5" }
```

Give both types an operator-facing `Display` in `cutplan`, next to the rules that produce them, and
let both callers wrap it instead of reimplementing it.

## Precedent

This shape is not new here. Candidate 1 shipped it for `TraceError`
(`crates/trace/src/lib.rs:171-200`): `Display` for the sentence, `std::error::Error` for the trait
bound, and `code() -> &'static str` for the caller that must branch on the *kind* of failure rather
than match its text across a language boundary. `apps/desktop/src/ipc.rs:246` reduces to one line:

```rust
fn trace_error(e: trace::TraceError) -> IpcError {
    IpcError::new(e.code(), e.to_string())
}
```

Candidate 6 is that same shape applied to a nested pair of enums, so `CutError::code()` delegates
`Preflight(e) => e.code()` and `CutError`'s `Display` delegates `Preflight(e) => write!(f, "{e}")`.

Codes are what makes the prose safe to change. `CutDialog.tsx:165` branches on `"stale_plan"` and
nothing else; no frontend, test, or e2e fake matches a message body. Every code below is the one
`device.rs` already emits, so the IPC contract is unchanged.

## Divergence from the review

**The CLI keeps one wording of its own: `NothingToCut`.** The review reads the CLI's five special
cases as duplication to be collapsed. Four of them are. The fifth is not: `"no cuttable paths in
SVG"` is a fact about the *input*, and `cutplan` cannot say it. By the time `preflight` runs, the
document is a `DocumentPasses` with no selected pass holding geometry — that it was imported at all,
let alone from an SVG with no strokes, is knowledge that lives in `pipeline.rs`. Collapsing that arm
would trade a sentence about the operator's file for a sentence about the planner's state.

Verified against the two tests that assert the string, because the review's file list implies both
are at risk and neither is:

- `pipeline.rs:349` (`plan_plain_cut`, empty SVG) never reaches `describe_cut_error`. It returns at
  `pipeline.rs:167`, the explicit empty-passes check. Untouched.
- `pipeline.rs:288` (`plan_cut_from_svg`, a fill-only rect) does reach it, and keeps its wording.

Net test churn from this design: zero.

**Plan assembly stays split, as the review's own caution note asks.** The two callers differ
substantively — the desktop passes `expect_revision: Some(_)`, hardcodes `allow_out_of_bounds:
false` and resolves `MaterialPreset`s per pass; the CLI passes `None`, exposes a flag, and applies
one `Settings` to every pass. A shared assembler would re-splay its arguments and own no rule.
Revisit at a third caller.

**No `thiserror`.** Not a workspace dependency, and `TraceError` hand-writes its impl. Twenty lines
beat a new dependency.

## Scope

In scope: `Display`, `std::error::Error` and `code()` on `CutError` and `PreflightError` in
`cutplan`; `device.rs`'s two mapping functions collapsed to one; `pipeline.rs`'s
`describe_cut_error` reduced to two arms.

Out of scope, decided deliberately:

- **`SettingsOutOfRange`'s text.** Its `&'static str` payloads (`"repeat_count must be 1..=10"`,
  `"speed must be 1..=30"`, `"force must be 1..=33"`) are already single-sourced in
  `preflight.rs:97-110` and passed straight through by both callers. Nothing is duplicated, so
  nothing needs moving. `Display` forwards the string unprefixed, the way `TraceError::Input` does.
- **Naming the offending node.** `preflight` holds `NodeId`s and no `Document`, so `shape #3` is the
  most it can say. Resolving that to a node's name needs a lookup the refusal doesn't have.
- **Localization.** One language, as everywhere else in this codebase.

## The ten refusals

`shape #{n}` renders `NodeId(n)` as its number — `NodeId` is a newtype over `u64` with no `Display`,
and `{:?}` in operator text is the thing this candidate exists to remove.

| variant | code | `Display` |
|---|---|---|
| `CutError::StalePlan` | `stale_plan` | the document changed since this cut was planned |
| `CutError::UnknownPassColor(Some(c))` | `unknown_pass_color` | no planned pass has color #RRGGBBAA |
| `CutError::UnknownPassColor(None)` | `unknown_pass_color` | no planned pass without a color |
| `NothingToCut` | `nothing_to_cut` | no pass selected for this cut has any geometry |
| `NonFiniteGeometry(n)` | `non_finite_geometry` | shape #3 has a coordinate that is not a finite number |
| `DegeneratePolyline(n)` | `degenerate_polyline` | shape #3 has a path with fewer than two points |
| `OutOfBounds { n, bounds }` | `out_of_bounds` | shape #3 lies outside the 305 x 305 mm cutting area |
| `SettingsOutOfRange(m)` | `settings_out_of_range` | *m*, verbatim |
| `MachineMismatch { d, m }` | `machine_mismatch` | this document is set up for `puma`, but the connected machine is `cameo5` |
| `OutputTooLarge(bytes)` | `output_too_large` | the encoded cut is about 96 MB, over the 64 MB limit |

Two wordings are chosen against what the code says today, for reasons worth recording:

`NothingToCut` avoids "enabled pass". `CONTEXT.md` lists *enabled pass* under `_Avoid_`, and
`CLAUDE.md` states the rule it protects: "A pass is not 'enabled/disabled': a colour nobody lists in
`PlanOptions::passes` is not cut." The desktop's current `"no enabled pass has any geometry"`
describes a flag that is not the mechanism.

`OutputTooLarge` reports megabytes rather than the raw byte count the variant carries. The number is
an estimate — `preflight.rs:127` weights 16 bytes per point by `repeat_count` — and printing an
estimate to the byte claims a precision it does not have.

## What each caller becomes

`device.rs` — 34 lines to 4, keeping the code table that `CutDialog` branches on and the comment
that says why it exists:

```rust
/// Every way `plan_cut` can refuse, as an IPC code the UI can branch on.
/// `stale_plan` is the one the frontend actually keys off (CutDialog.tsx).
fn map_cut_error(e: CutError) -> IpcError {
    IpcError::new(e.code(), e.to_string())
}
```

`pipeline.rs` — 14 lines to 8, one arm surviving per the divergence above and one augmenting the
shared sentence with the flag that overrules it:

```rust
/// `CutError` as something to print at a terminal. Two arms outlive the shared
/// `Display`: `NothingToCut` because only this caller knows an SVG was imported,
/// and out-of-bounds because naming `--allow-out-of-bounds` is the CLI's to do —
/// the desktop hardcodes `allow_out_of_bounds: false` and offers no such control.
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

The desktop's `out_of_bounds` message improves as a side effect. Today it prints `node NodeId(3)
outside (0.0, 0.0, 305.0, 305.0)` — the terser and more `Debug`-laden of the two, for the one
refusal where the CLI already wrote a good sentence.

## Testing

In `cutplan`, one test per type asserting the whole table — every variant's `Display` and every
variant's `code()` — so adding a variant without a sentence fails to compile the match and adding
one with the wrong sentence fails the assertion.

In `pipeline.rs`, the existing `an_svg_with_nothing_stroked_is_refused_by_name` keeps `NothingToCut`
honest. Add two against `describe_cut_error` directly — a `MachineMismatch` reading as a sentence
rather than a struct literal, which is the specific leak this candidate fixes, and an out-of-bounds
naming the flag that overrules it. Called directly rather than driven through `plan_cut_from_svg`,
because an SVG import never sets a machine id, so no CLI path can reach `MachineMismatch` at all.

In `device.rs`, the existing `prepare_cut` tests assert codes and keep doing so; no new test, since
`map_cut_error` no longer holds a decision.

## Risks

The messages are user-visible strings with no schema, so a future change can silently reword a
refusal an operator has learned. The codes are the stable half of the contract and stay pinned by
the `cutplan` table test and by `CutDialog`'s `stale_plan` branch.
