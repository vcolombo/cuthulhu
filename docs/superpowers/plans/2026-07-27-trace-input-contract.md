<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Trace Input Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `trace` exports an interface in the units its users think in — a control table, user-facing `detail`, its own file-reading ceiling, and typed error codes — so the CLI and the desktop stop restating what the module knows.

**Architecture:** Today `trace`'s public surface is vtracer's parameter list, so both callers translate it and each picked a different translation. Move the whole input contract behind the seam: a `CONTROLS` spec table that `validate`, clap, and the dialog all read; a `TraceControls` type in user units with the detail inversion happening inside `trace`; a `trace::read_image` that owns the 256 MiB ceiling next to `MAX_DECODE_ALLOC`; and `TraceError::code()` so callers branch on a code instead of matching a `Display` string. The vtracer vocabulary stops at the module boundary.

**Tech Stack:** Rust (serde, clap 4 derive, Tauri v2 commands), TypeScript/React, vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-07-27-trace-input-contract-design.md`

## Global Constraints

- SPDX header on every file: `// SPDX-License-Identifier: GPL-3.0-or-later` (or the language's comment form).
- Comments explain *why*, not *what*. Do not add restating comments. The existing comments in `ipc.rs` and `trace/src/lib.rs` document traps — when code moves, **its comments move with it, verbatim unless the reason changed**.
- `cargo test --workspace --locked` is what CI runs. `--locked` is mandatory.
- **`Cargo.lock` changes in Task 1 and must be committed with it.** `tempfile` moves from `apps/desktop`'s dev-dependencies to `crates/trace`'s. This is the opposite of the machine-caps change — do not assume an unchanged lock means success here.
- `apps/desktop/ui/dist/` is committed. Any change under `ui/src` requires `npm --prefix apps/desktop/ui run build` and committing the rebuilt `dist/` in the same change, or CI fails.
- Vocabulary from `CONTEXT.md` is normative: **Trace**, **TraceControls** (added by Task 2), **Document**, **Node**. Never write "vectorize", "autotrace", or "raster conversion" for Trace, and never "options"/"params"/"settings" for TraceControls.
- Commit subjects: imperative with the reason attached.
- **Never name a vtracer parameter outside `crates/trace/src/lib.rs`.** `filter_speckle`, `corner_threshold`, `length_threshold`, and `color_precision` must not appear in `crates/cli`, `apps/desktop`, or any `.ts`/`.tsx` file once Task 3 is done. That absence is the deliverable.

---

## File Structure

| File | Responsibility after this change |
| --- | --- |
| `crates/trace/src/lib.rs` | The single statement of every trace input: the `CONTROLS` table, `TraceControls` in user units, the detail inversion, the 256 MiB file ceiling, and `TraceError::code()` |
| `crates/trace/Cargo.toml` | Gains `tempfile` as a dev-dependency for the file-ceiling test |
| `crates/trace/tests/roundtrip.rs` | Same assertions, rebuilt on `TraceControls` |
| `crates/cli/src/main.rs` | Reads the table for clap defaults and help; owns no ceiling, no ranges, no error rewording |
| `crates/cli/tests/trace.rs` | Unchanged assertions plus one for the flipped `--detail` |
| `apps/desktop/Cargo.toml` | Loses the now-unused `tempfile` dev-dependency |
| `apps/desktop/src/ipc.rs` | Authorization only; the trace commands become three lines each |
| `apps/desktop/src/main.rs` | Command registration |
| `apps/desktop/ui/src/ipc.ts` | Trace wire types in user units; `traceControls()` |
| `apps/desktop/ui/src/trace/viewmodel.ts` | Preview state and staleness only — no conversion, no defaults |
| `apps/desktop/ui/src/trace/TraceDialog.tsx` | Renders sliders from the table it fetched |
| `apps/desktop/ui/e2e/smoke.spec.ts` | Fake answers `trace_controls` with a fixture |
| `CHANGELOG.md` | New. Records the `--detail` flip |

---

## One deviation from the spec, decided at plan time

The spec sketches a private `read_image_capped(path, cap)` so the ceiling can be tested with a
handful of bytes. The existing tests being moved already cover the bound twice and more cheaply —
`read_capped(reader, cap)` at the reader level with `std::io::repeat`, and the real
`MAX_INPUT_FILE_BYTES` at the file level with a sparse file. A third parameterized entry point earns
nothing over those two, so **`read_capped` moves as-is and `read_image_capped` is not written.**
`read_image(path)` uses the constant directly.

---

## Task 1: `trace` owns the input ceiling

Purely additive to `trace`'s surface — no signature changes, no behaviour change, no wire change.
Both binaries delete their copies. Workspace stays green throughout; the app is unaffected.

**Files:**
- Modify: `crates/trace/src/lib.rs` (add after `MAX_DECODE_ALLOC` at line 10, and a new `TraceError` variant at line 33)
- Modify: `crates/trace/Cargo.toml`
- Modify: `crates/cli/src/main.rs:10-44` (delete), `:189` (call site)
- Modify: `apps/desktop/src/ipc.rs:174-177,231-278` (delete), `:287,298` (call sites), `:317-355` (tests move out)
- Modify: `apps/desktop/Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: nothing (first task).
- Produces:
  - `pub const trace::MAX_INPUT_FILE_BYTES: u64`
  - `pub fn trace::read_image(path: &std::path::Path) -> Result<Vec<u8>, TraceError>`
  - `TraceError::Input(String)` — a new variant; `Display` renders the message unchanged.

- [ ] **Step 1: Move the three ceiling tests into `trace`**

Append to the `mod tests` block in `crates/trace/src/lib.rs`, keeping every doc comment verbatim —
they are the record of *why* the code is shaped this way:

```rust
    /// The ceiling has to come from the read itself. A separate size check describes whatever the
    /// pathname pointed at when it ran, so a file that grows between the check and the read is
    /// read in full despite having just passed the limit — the cap is advisory rather than a
    /// bound. Deliberately exercised through a plain reader, with no file and no metadata call,
    /// because that is the property under test.
    #[test]
    fn read_capped_refuses_a_stream_longer_than_the_cap() {
        use std::io::Read as _;
        let over = std::io::repeat(b'x').take(9);
        assert!(read_capped(over, 8).unwrap().is_none(), "9 bytes must be refused against a cap of 8");
    }

    /// Covers the glue the helper tests cannot: opening the path, threading the real ceiling
    /// through, and handing back the bytes. Both trace entry points read through here, so a
    /// mistake in this wiring breaks every trace.
    #[test]
    fn read_image_returns_the_contents_of_a_small_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.bin");
        std::fs::write(&path, b"not an image, but bytes are bytes").unwrap();
        assert_eq!(read_image(&path).unwrap(), b"not an image, but bytes are bytes");
    }

    /// Exercises the real `MAX_INPUT_FILE_BYTES`, which the `read_capped` tests deliberately do
    /// not. The file is extended rather than written, so no quarter gigabyte ever moves through
    /// this process. What it costs on disk is the filesystem's business — usually nothing, since
    /// the range can be left unallocated, but `set_len` promises the size and never the storage.
    #[test]
    fn read_image_refuses_a_file_past_the_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.bin");
        std::fs::File::create(&path).unwrap().set_len(MAX_INPUT_FILE_BYTES + 1).unwrap();
        assert!(matches!(read_image(&path), Err(TraceError::Input(m)) if m.contains("too large")));
    }

    /// A path that does not exist is an input failure, not a decode failure: nothing was ever
    /// handed to the decoder. The distinction is what `code()` will make visible to the desktop.
    #[test]
    fn read_image_reports_a_missing_file_as_input() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(read_image(&dir.path().join("nope.png")), Err(TraceError::Input(_))));
    }
```

Add the dev-dependency in `crates/trace/Cargo.toml`:

```toml
[dev-dependencies]
fileio = { path = "../fileio" }
tempfile = "3"
```

- [ ] **Step 2: Run the tests to verify they fail**

```sh
cargo test -p trace read_
```

Expected: FAIL to compile — `cannot find function 'read_capped'`, `cannot find function 'read_image'`, `no variant named 'Input'`.

- [ ] **Step 3: Add the variant, the constant, and the two functions**

In `crates/trace/src/lib.rs`, extend the error enum (line 33) and its `Display`:

```rust
#[derive(Debug, PartialEq)]
pub enum TraceError { Input(String), InvalidOption(String), Decode(String), Trace(String), EmptyResult }
```

```rust
            TraceError::Input(m) => write!(f, "{m}"),
```

`Input`'s messages are already whole sentences naming the file, so a prefix would read twice. Add
that as the comment above the arm.

Insert after `MAX_DECODE_ALLOC` (line 10):

```rust
/// Ceiling on the source *file*, as opposed to `MAX_DECODE_ALLOC`'s ceiling on what decoding it may
/// allocate. The decoder's limit only applies once the bytes are already resident, so without this
/// a huge file exhausts memory before it can be rejected for not being a usable image.
pub const MAX_INPUT_FILE_BYTES: u64 = 256 * 1024 * 1024;

fn too_large() -> TraceError {
    TraceError::Input(format!(
        "file is too large to open: over {} MiB",
        MAX_INPUT_FILE_BYTES / (1024 * 1024)
    ))
}

/// Read a whole stream, refusing input longer than `cap` bytes.
///
/// `cap` is a parameter rather than the constant so the bound can be exercised with a handful of
/// bytes instead of a quarter gigabyte.
fn read_capped<R: std::io::Read>(reader: R, cap: u64) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    // One byte past the ceiling, so landing exactly on it is distinguishable from exceeding it,
    // and so an oversized input costs one extra byte rather than its whole length.
    reader.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Ok(None);
    }
    Ok(Some(bytes))
}

/// Read an image file, refusing anything past the ceiling.
///
/// Everything happens through one open handle, rather than `std::fs::metadata` followed by a
/// separate `std::fs::read`. Two *pathname* resolutions describe two moments: the size that passed
/// the check belonged to whatever the path pointed at then, and a file that grew in between was
/// read in full anyway.
///
/// The size check itself is not the problem and is kept — `File::metadata` is `fstat` on the
/// handle, so it cannot describe a different file than the one about to be read. It earns its
/// place by refusing an oversized file for the cost of a syscall, instead of allocating the whole
/// ceiling first only to throw it away.
///
/// Takes the path the caller means to open, and opens exactly that. The desktop authorizes first
/// and passes the already-canonical path it got back, so its check and this open are one
/// resolution; handing an unresolved path here instead would reopen the window
/// `apps/desktop/src/ipc.rs`'s `authorized_path` exists to close.
pub fn read_image(path: &std::path::Path) -> Result<Vec<u8>, TraceError> {
    let file = std::fs::File::open(path)
        .map_err(|e| TraceError::Input(format!("cannot read {}: {e}", path.display())))?;
    // A failed fstat is not fatal: this is a fast path, and `read_capped` below is the real bound.
    if file.metadata().is_ok_and(|m| m.len() > MAX_INPUT_FILE_BYTES) {
        return Err(too_large());
    }
    match read_capped(file, MAX_INPUT_FILE_BYTES)
        .map_err(|e| TraceError::Input(format!("cannot read {}: {e}", path.display())))?
    {
        Some(bytes) => Ok(bytes),
        None => Err(too_large()),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```sh
cargo test -p trace
```

Expected: PASS, including the four new tests.

- [ ] **Step 5: Point the CLI at it**

In `crates/cli/src/main.rs`, delete lines 10–44 entirely — the `MAX_INPUT_FILE_BYTES` constant, its
doc comment, and the whole `read_image_capped` function. Replace the call at line 189:

```rust
            let bytes = trace::read_image(&file).map_err(|e| e.to_string())?;
```

- [ ] **Step 6: Point the desktop at it**

In `apps/desktop/src/ipc.rs`, delete the `MAX_INPUT_FILE_BYTES` constant and its doc comment
(lines 174–177), and the `read_capped`, `too_large`, and `read_image_file` functions
(lines 231–278). Delete the three tests that moved to `trace` (lines 317–355), and `tempfile` from
`apps/desktop/Cargo.toml`'s `[dev-dependencies]`.

Keep `pick_image`, `AuthorizedImages`, `canonical`, and `authorized_path` exactly as they are.

Replace both call sites:

```rust
    let real = authorized_path(&auth, &path)?;
    let bytes = trace::read_image(&real).map_err(|e| e.to_string())?;
```

- [ ] **Step 7: Run the whole workspace**

```sh
cargo test --workspace --locked
```

Expected: FAIL on `--locked`, because moving `tempfile` between crates changes `Cargo.lock`. Run
`cargo test --workspace` once to let it update, confirm the only lock changes are `tempfile` moving
between the `trace` and `cuthulhu-desktop` package entries, then re-run with `--locked`.

Expected after that: PASS, including `crates/cli/tests/trace.rs`'s
`trace_refuses_a_file_larger_than_the_read_cap`, which now exercises the moved code unchanged.

- [ ] **Step 8: Commit**

```sh
git add crates/trace/src/lib.rs crates/trace/Cargo.toml crates/cli/src/main.rs \
        apps/desktop/src/ipc.rs apps/desktop/Cargo.toml Cargo.lock
git commit -m "Let trace own the ceiling on the files it reads, since neither binary did

The 256 MiB input cap was implemented twice, in full, down to the error text
and the argument in the comments. It belongs to neither entry point: it exists
to bound trace's decoder, so it now sits beside MAX_DECODE_ALLOC."
```

---

## Task 2: The control table, in the units people use

`trace()`'s signature changes here, so `trace`, `crates/cli`, and `apps/desktop` cannot move
independently — this is one commit. There is an intermediate checkpoint at `cargo test -p trace`
before the callers are updated; the workspace will not build between Steps 4 and 8, which is
expected.

**Files:**
- Modify: `crates/trace/src/lib.rs:12-70,227-270` and its `mod tests`
- Modify: `crates/trace/tests/roundtrip.rs:2,20-22`
- Modify: `crates/cli/src/main.rs:92-114,176-198`
- Modify: `crates/cli/tests/trace.rs`
- Modify: `apps/desktop/src/ipc.rs` (the two trace commands), `apps/desktop/src/main.rs:62`
- Create: `CHANGELOG.md`
- Modify: `CONTEXT.md`

**Interfaces:**
- Consumes: `trace::read_image`, `TraceError::Input` (Task 1).
- Produces:
  - `pub struct trace::ControlSpec { name, label, help: &'static str, min, max, step, default: f64, color_only: bool }`
  - `pub const trace::{SPECKLE, SMOOTHING, DETAIL, COLORS}: ControlSpec`
  - `pub const trace::CONTROLS: [ControlSpec; 4]`
  - `pub struct trace::TraceControls { mode: TraceMode, speckle: u8, smoothing: u8, detail: f64, colors: u8 }`, `impl Default`
  - `pub struct trace::TraceControlSpecs { controls: [ControlSpec; 4], default_mode: TraceMode, max_dim: u32 }`
  - `pub fn trace::control_specs() -> TraceControlSpecs`
  - `pub fn trace::trace(image_bytes: &[u8], controls: &TraceControls) -> Result<TraceResult, TraceError>` — **replaces** the `TraceOptions` signature
  - `pub fn TraceError::code(&self) -> &'static str`
  - Tauri command `trace_controls() -> Result<TraceControlSpecs, IpcError>`
  - Tauri command `trace_image(path: PathBuf, controls: TraceControls) -> Result<TraceResult, IpcError>` (was `opts: TraceOptions`, `Err = String`)

- [ ] **Step 1: Write the failing tests for the table**

Add to `mod tests` in `crates/trace/src/lib.rs`:

```rust
    /// The ranges are data now, not prose in four places. Each control is rejected one step
    /// outside the bounds its own spec states, so a range can only be changed in the table.
    #[test]
    fn validate_rejects_each_control_outside_its_own_spec() {
        let over = |c: &TraceControls| validate(c).unwrap_err();
        let d = TraceControls::default();

        let bad = TraceControls { speckle: SPECKLE.max as u8 + 1, ..d.clone() };
        assert!(matches!(over(&bad), TraceError::InvalidOption(m) if m.contains("speckle")));
        let bad = TraceControls { smoothing: SMOOTHING.max as u8 + 1, ..d.clone() };
        assert!(matches!(over(&bad), TraceError::InvalidOption(m) if m.contains("smoothing")));
        let bad = TraceControls { detail: DETAIL.max + DETAIL.step, ..d.clone() };
        assert!(matches!(over(&bad), TraceError::InvalidOption(m) if m.contains("detail")));
        let bad = TraceControls { detail: DETAIL.min - DETAIL.step, ..d.clone() };
        assert!(matches!(over(&bad), TraceError::InvalidOption(m) if m.contains("detail")));
        let bad = TraceControls { colors: COLORS.min as u8 - 1, ..d.clone() };
        assert!(matches!(over(&bad), TraceError::InvalidOption(m) if m.contains("colors")));

        assert!(validate(&d).is_ok());
    }

    /// clap's derive needs a literal for `help`, so each spec states its range in prose beside the
    /// numbers. That is the one restatement this design accepts, and only because this test makes
    /// it impossible to change one without the other.
    #[test]
    fn control_help_states_its_own_range() {
        fn rendered(v: f64) -> String { format!("{v}") }
        for spec in CONTROLS {
            assert!(
                spec.help.contains(&rendered(spec.min)) && spec.help.contains(&rendered(spec.max)),
                "{}: help {:?} does not state its range {}–{}",
                spec.name, spec.help, spec.min, spec.max,
            );
        }
    }

    /// Detail is reflected through its own range rather than subtracted from a constant. The
    /// constant that used to live in the UI (`13.5`) was only correct because 3.5 + 10 happens to
    /// equal it; moving either bound would have broken the mapping silently.
    #[test]
    fn detail_reflects_through_its_own_range() {
        assert_eq!(length_threshold(DETAIL.max), DETAIL.min);
        assert_eq!(length_threshold(DETAIL.min), DETAIL.max);
        // The default trace is unchanged by the flip: 9.5 user-facing is vtracer's old 4.0 default.
        assert_eq!(length_threshold(DETAIL.default), 4.0);
    }

    /// A caller that must branch on the kind of failure gets a code, not a `Display` string. The
    /// dialog's empty state used to be selected by matching the literal "empty".
    #[test]
    fn every_error_carries_a_code() {
        assert_eq!(TraceError::Input(String::new()).code(), "input");
        assert_eq!(TraceError::InvalidOption(String::new()).code(), "invalid_option");
        assert_eq!(TraceError::Decode(String::new()).code(), "decode");
        assert_eq!(TraceError::Trace(String::new()).code(), "trace");
        assert_eq!(TraceError::EmptyResult.code(), "empty");
    }

    /// One sentence, printed verbatim by both entry points. Before this, the CLI said "lower
    /// --detail" and the dialog said "raise detail" for the same failure, and both were right.
    #[test]
    fn empty_result_names_both_ways_out() {
        let m = TraceError::EmptyResult.to_string();
        assert!(m.contains("speckle") && m.contains("detail"), "{m}");
    }
```

Replace the existing `validate_rejects_out_of_range` and `empty_result_displays_empty_sentinel`
tests — the first is subsumed by the table-driven version, and the second asserted the sentinel that
is being removed.

- [ ] **Step 2: Run to verify they fail**

```sh
cargo test -p trace
```

Expected: FAIL to compile — `cannot find type 'TraceControls'`, `cannot find value 'SPECKLE'`,
`cannot find function 'length_threshold'`, `no method named 'code'`.

- [ ] **Step 3: Write the table and the controls type**

Replace `crates/trace/src/lib.rs:12-30` (the `TraceOptions` block, keeping `TraceMode`) with:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TraceMode { Binary, Color }

/// One user-facing trace control: what it is called, what it accepts, and where it starts.
///
/// The single statement of these numbers. `validate` builds its refusals from them, the CLI takes
/// its clap defaults and help from them, and the desktop ships the table to the dialog, which
/// renders its sliders from it — so a range moves in one place instead of four.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
    /// Inert in binary mode. Which controls those are is the tracer's knowledge, not the widget's.
    pub color_only: bool,
}

pub const SPECKLE: ControlSpec = ControlSpec {
    name: "speckle", label: "Ignore speckles",
    help: "Ignore speckles up to this size in px (0–16)",
    min: 0.0, max: 16.0, step: 1.0, default: 4.0, color_only: false,
};
pub const SMOOTHING: ControlSpec = ControlSpec {
    name: "smoothing", label: "Smoothing",
    help: "Corner threshold in degrees (0–180); higher = smoother",
    min: 0.0, max: 180.0, step: 1.0, default: 60.0, color_only: false,
};
pub const DETAIL: ControlSpec = ControlSpec {
    name: "detail", label: "Detail",
    help: "Level of detail (3.5–10); higher = more detail",
    min: 3.5, max: 10.0, step: 0.5, default: 9.5, color_only: false,
};
pub const COLORS: ControlSpec = ControlSpec {
    name: "colors", label: "Colors",
    help: "Color precision in bits (1–8, color mode only)",
    min: 1.0, max: 8.0, step: 1.0, default: 6.0, color_only: true,
};

/// In display order.
pub const CONTROLS: [ControlSpec; 4] = [SPECKLE, SMOOTHING, DETAIL, COLORS];

/// What a caller asks for, in the units a person setting it thinks in.
///
/// vtracer's own parameter names and directions do not appear here, or anywhere outside this
/// module: they are an implementation of tracing, not a description of it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceControls {
    pub mode: TraceMode,
    pub speckle: u8,
    pub smoothing: u8,
    /// Higher means more detail, the direction a person expects from a control with this name.
    pub detail: f64,
    pub colors: u8,
}

impl Default for TraceControls {
    fn default() -> Self {
        TraceControls {
            mode: TraceMode::Binary,
            speckle: SPECKLE.default as u8,
            smoothing: SMOOTHING.default as u8,
            detail: DETAIL.default,
            colors: COLORS.default as u8,
        }
    }
}

/// The table plus the two facts that are not controls: what mode a fresh dialog starts in, and the
/// size a large image is reduced to. `MAX_DIM` travels with them because the dialog states it to
/// the user, and used to state it as a hardcoded 2048.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceControlSpecs {
    pub controls: [ControlSpec; 4],
    pub default_mode: TraceMode,
    pub max_dim: u32,
}

pub fn control_specs() -> TraceControlSpecs {
    TraceControlSpecs {
        controls: CONTROLS,
        default_mode: TraceControls::default().mode,
        max_dim: MAX_DIM,
    }
}

/// vtracer's `length_threshold` runs opposite to detail: a lower threshold keeps shorter segments,
/// so it yields *more* detail. Reflect the user's value through its own range rather than
/// subtracting a constant, so moving either bound cannot silently break the mapping.
fn length_threshold(detail: f64) -> f64 {
    DETAIL.min + DETAIL.max - detail
}
```

Replace `validate` (lines 56–70):

```rust
/// Refuse a control that falls outside the range its own spec states.
pub(crate) fn validate(c: &TraceControls) -> Result<(), TraceError> {
    for (spec, value) in [
        (SPECKLE, c.speckle as f64),
        (SMOOTHING, c.smoothing as f64),
        (DETAIL, c.detail),
        (COLORS, c.colors as f64),
    ] {
        // A NaN fails `contains` too, which is the answer we want and the reason this is not
        // written as a pair of comparisons.
        if !(spec.min..=spec.max).contains(&value) {
            return Err(TraceError::InvalidOption(format!(
                "{} must be {}–{}", spec.name, spec.min, spec.max
            )));
        }
    }
    Ok(())
}
```

Add `code()` beside the `Display` impl, and change the `EmptyResult` arm:

```rust
impl TraceError {
    /// Stable identifier for a caller that must branch on the *kind* of failure rather than show
    /// its text — the desktop sends it as `IpcError::code`, and the dialog renders its empty state
    /// from `"empty"` instead of matching a `Display` string across a language boundary.
    pub fn code(&self) -> &'static str {
        match self {
            TraceError::Input(_) => "input",
            TraceError::InvalidOption(_) => "invalid_option",
            TraceError::Decode(_) => "decode",
            TraceError::Trace(_) => "trace",
            TraceError::EmptyResult => "empty",
        }
    }
}
```

```rust
            TraceError::EmptyResult =>
                write!(f, "nothing traced — lower the speckle filter or raise detail"),
```

- [ ] **Step 4: Rewrite `trace()`'s signature and its vtracer conversion**

At `crates/trace/src/lib.rs:227`, change the signature and the `Config` construction. Everything
between them — the transparency handling, the padding, the panic containment — is untouched:

```rust
pub fn trace(image_bytes: &[u8], controls: &TraceControls) -> Result<TraceResult, TraceError> {
    validate(controls)?;
```

```rust
    let config = vtracer::Config {
        color_mode: match controls.mode {
            TraceMode::Binary => vtracer::ColorMode::Binary,
            TraceMode::Color => vtracer::ColorMode::Color,
        },
        hierarchical: vtracer::Hierarchical::Stacked,
        filter_speckle: controls.speckle as usize,
        color_precision: controls.colors as i32,
        corner_threshold: controls.smoothing as i32,
        length_threshold: length_threshold(controls.detail),
        ..vtracer::Config::default()
    };
```

Replace the remaining `opts.mode` reads inside the function (the binary-mode flatten at line 242 and
the colour-mode padding at line 250) with `controls.mode`.

Update every existing test in `mod tests` that builds `TraceOptions { .. }` to build
`TraceControls { .. }`. The rename is mechanical and complete: `TraceOptions` → `TraceControls`,
`filter_speckle:` → `speckle:`. No other field in those tests changes name, and no value changes —
`speckle_filter_can_empty_the_result` keeps `16`; the checkerboard and transparency tests keep `0`.

Confirm none were missed before moving on:

```sh
grep -rn "TraceOptions\|filter_speckle" crates/trace/
```

Expected: no matches.

- [ ] **Step 5: Run the trace crate's tests**

```sh
cargo test -p trace
```

Expected: PASS. The workspace as a whole will not build until Step 8 — that is expected, and is why
this task is one commit.

- [ ] **Step 6: Update the roundtrip test**

In `crates/trace/tests/roundtrip.rs`, line 2 and lines 20–22:

```rust
use trace::{trace, TraceControls, TraceMode};
```

```rust
        let controls = TraceControls { mode, ..TraceControls::default() };
        let r = trace(&png_black_square(), &controls).unwrap();
```

- [ ] **Step 7: Write the CLI's failing test for the flip**

`fixture_png` is a hard-edged square: vtracer traces its corners with `L` commands at every
`--detail` value, so counting `L` commands cannot observe the control at all — the assertion
`commands("10") >= commands("3.5")` is `0 >= 0` on that fixture and passes even with the inversion
deleted, reversed, or replaced by a constant. Use a filled circle instead, which has no corners and
is traced entirely with curve (`C`) commands, and assert **strictly** (`>`, not `>=`): `>=` is what
let the dead control through in the first place.

Add to `crates/cli/tests/trace.rs`, alongside `fixture_png`:

```rust
/// A filled circle, unlike `fixture_png`'s square, has no corners — it is traced entirely with
/// curve (`C`) commands at every detail level, so the curve count is what actually moves when
/// `--detail` moves. A polygon's corners are corners regardless of threshold and cannot observe
/// the control at all.
fn fixture_circle_png(dir: &std::path::Path) -> std::path::PathBuf {
    let (cx, cy, r) = (100.0_f64, 100.0_f64, 70.0_f64);
    let img = image::RgbaImage::from_fn(200, 200, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
        if dx * dx + dy * dy <= r * r {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    });
    let p = dir.join("circle.png");
    img.save(&p).unwrap();
    p
}

/// `--detail` is stated in the same units as the desktop's Detail slider: higher means more
/// detail. It used to carry vtracer's `length_threshold`, which runs the other way, so the two
/// interfaces printed opposite advice for the same failure. This asserts strictly (`>`, not `>=`)
/// on purpose: `>=` also passes when the control does nothing at all, which is the failure this
/// test exists to catch — a hard-edged square fixture traces with zero curve commands at every
/// detail level, so `>=` on that fixture cannot distinguish "detail works" from "detail is dead".
#[test]
fn detail_reads_high_for_more_detail() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_circle_png(dir.path());
    let curve_commands = |detail: &str| {
        let out = dir.path().join(format!("out-{detail}.svg"));
        let status = bin().args([
            "trace", input.to_str().unwrap(), "-o", out.to_str().unwrap(), "--detail", detail,
        ]).status().unwrap();
        assert!(status.success(), "--detail {detail} failed");
        std::fs::read_to_string(&out).unwrap().matches('C').count()
    };
    assert!(
        curve_commands("10") > curve_commands("3.5"),
        "higher --detail must trace strictly more curve commands"
    );
}
```

Expected on this fixture: `--detail 3.5` → 7 curve commands (504 bytes of SVG); `--detail 10` → 12
curve commands (696 bytes). The pixel-center offset (`x as f64 + 0.5 - cx`) matters: an
integer-coordinate circle mask traces to the same curve count at both detail values on this image
size, so it would silently reintroduce the same non-discriminating test it replaces.

- [ ] **Step 8: Update the CLI**

In `crates/cli/src/main.rs`, replace the `Trace` variant's four numeric fields (lines 102–113).
Doc comments come off those fields: clap gives an explicit `help` attribute priority over a doc
comment, so leaving both would put the help text in two places again.

```rust
        /// binary (single-color silhouette) or color (one path per color cluster)
        #[arg(long, default_value = "binary")]
        mode: String,
        #[arg(long, help = trace::SPECKLE.help, default_value_t = trace::SPECKLE.default as u8)]
        speckle: u8,
        #[arg(long, help = trace::SMOOTHING.help, default_value_t = trace::SMOOTHING.default as u8)]
        smoothing: u8,
        #[arg(long, help = trace::DETAIL.help, default_value_t = trace::DETAIL.default)]
        detail: f64,
        #[arg(long, help = trace::COLORS.help, default_value_t = trace::COLORS.default as u8)]
        colors: u8,
```

Replace the command body (lines 176–198):

```rust
        Command::Trace { file, output, mode, speckle, smoothing, detail, colors } => {
            let mode = match mode.as_str() {
                "binary" => trace::TraceMode::Binary,
                "color" => trace::TraceMode::Color,
                other => return Err(format!("--mode must be binary or color, got {other}")),
            };
            let controls = trace::TraceControls { mode, speckle, smoothing, detail, colors };
            let bytes = trace::read_image(&file).map_err(|e| e.to_string())?;
            let result = trace::trace(&bytes, &controls).map_err(|e| e.to_string())?;
            std::fs::write(&output, result.svg)
                .map_err(|e| format!("cannot write {}: {e}", output.display()))?;
            println!("{} paths → {}", result.path_count, output.display());
            // The reduction has always happened and was never reported here, so a large image
            // traced at a size the operator did not choose and had no way to notice.
            if result.downscaled {
                println!("large image reduced to {} px for tracing", trace::MAX_DIM);
            }
            Ok(())
        }
```

The `map_err` that reworded `EmptyResult` is gone: `trace` now says the same sentence to both
interfaces.

- [ ] **Step 9: Update the desktop backend**

In `apps/desktop/src/ipc.rs`, replace the two trace commands:

```rust
/// The tracer's own description of what it accepts, so the dialog renders its controls from the
/// module that enforces them rather than from a table typed to agree.
#[tauri::command]
pub fn trace_controls() -> Result<trace::TraceControlSpecs, IpcError> {
    Ok(trace::control_specs())
}

#[tauri::command(async)]
pub fn trace_image(
    auth: tauri::State<AuthorizedImages>,
    path: PathBuf,
    controls: trace::TraceControls,
) -> Result<trace::TraceResult, IpcError> {
    let real = authorized_path(&auth, &path).map_err(|m| IpcError::new("input", m))?;
    let bytes = trace::read_image(&real).map_err(trace_error)?;
    trace::trace(&bytes, &controls).map_err(trace_error)
}

/// Carry the tracer's own code across IPC, so the dialog branches on the kind of failure instead
/// of matching the text of one.
fn trace_error(e: trace::TraceError) -> IpcError {
    IpcError::new(e.code(), e.to_string())
}
```

`IpcError::new` is private to `apps/desktop/src/device.rs`; make it `pub(crate)` and add `IpcError`
to this file's existing import from `crate::device`.

Leave `load_image_preview` returning `Result<String, String>` — the thumbnail has no code to branch
on, and widening it is not this change.

Register the new command in `apps/desktop/src/main.rs`, after `ipc::trace_image` (line 62):

```rust
            ipc::trace_controls,
```

- [ ] **Step 10: Run the whole workspace**

```sh
cargo test --workspace --locked
```

Expected: PASS, including `detail_reads_high_for_more_detail` and the three pre-existing CLI trace
tests. `trace_reports_empty_result_without_writing` asserts `stderr` contains "nothing traced",
which the new sentence still satisfies.

- [ ] **Step 11: Write the CHANGELOG**

Create `CHANGELOG.md` at the repository root:

```markdown
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Changelog

Notable changes to Cuthulhu. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Nothing has been released yet, so there is no history before `Unreleased`.

## [Unreleased]

### Changed

- `cuthulhu trace --detail` is now stated in the same units as the desktop's Detail slider: **higher
  means more detail**. It previously carried vtracer's `length_threshold` verbatim, which runs the
  other way, so the two interfaces used one word for opposite things and printed opposite advice for
  the same failure. The default is unchanged in effect — the old `4.0` is the new `9.5`. A script
  passing `--detail` explicitly will trace differently and should be updated by reflecting its value
  through the range: `13.5 - old`.

### Added

- `cuthulhu trace` reports when a large image was reduced to 2048 px for tracing. The reduction
  always happened; it was only ever visible in the desktop.
```

- [ ] **Step 12: Add TraceControls to CONTEXT.md**

Insert after the **Trace** entry (`CONTEXT.md:25-28`), matching the file's existing format:

```markdown
**TraceControls**:
What a caller asks a Trace for, in the units a person setting them thinks in — mode, speckle,
smoothing, detail, colors. Ranges and defaults are stated once, in `trace::CONTROLS`. Detail rises
with detail; vtracer's inverse `length_threshold` never leaves the `trace` crate.
_Avoid_: options, params, settings (Settings is the machine's, not the tracer's), vtracer parameter
names
```

- [ ] **Step 13: Commit**

```sh
git add crates/trace crates/cli apps/desktop/src CHANGELOG.md CONTEXT.md
git commit -m "Give trace a contract in user units, so both callers stop translating it

Ranges and defaults were stated three times each and the CLI and the dialog
disagreed on which way detail runs — the same failure printed 'lower --detail'
in one and 'raise detail' in the other, both correct. trace now owns one table
and one direction, and --detail changes meaning to match the dialog.

Callers branch on TraceError::code() rather than matching a Display string."
```

---

## Task 3: The dialog reads the table

**Files:**
- Modify: `apps/desktop/ui/src/ipc.ts:228-245`
- Modify: `apps/desktop/ui/src/trace/viewmodel.ts:3-17,19-38`
- Modify: `apps/desktop/ui/src/trace/viewmodel.test.ts`
- Modify: `apps/desktop/ui/src/trace/TraceDialog.tsx:1-7,57,88-90,96-103,124,132,145-150`
- Modify: `apps/desktop/ui/e2e/smoke.spec.ts:143-151`
- Modify: `apps/desktop/ui/dist/` (rebuilt)

**Interfaces:**
- Consumes: `trace_controls` and the reshaped `trace_image` from Task 2.
- Produces: nothing downstream.

- [ ] **Step 1: Write the failing viewmodel tests**

Replace the `toOptionsDto` describe block in `apps/desktop/ui/src/trace/viewmodel.test.ts` — that
block existed to pin a conversion, and the conversion is gone. Do not port its `13.5` assertion; the
invariant it stood for is covered by `detail_reflects_through_its_own_range` in Rust.

```ts
import { acceptError, acceptResult, controlsFromSpecs, makeDebouncer, svgDataUrl } from "./viewmodel";

const specs = {
  controls: [
    { name: "speckle" as const, label: "Ignore speckles", help: "", min: 0, max: 16, step: 1, default: 4, colorOnly: false },
    { name: "smoothing" as const, label: "Smoothing", help: "", min: 0, max: 180, step: 1, default: 60, colorOnly: false },
    { name: "detail" as const, label: "Detail", help: "", min: 3.5, max: 10, step: 0.5, default: 9.5, colorOnly: false },
    { name: "colors" as const, label: "Colors", help: "", min: 1, max: 8, step: 1, default: 6, colorOnly: true },
  ],
  defaultMode: "binary" as const,
  maxDim: 2048,
};

describe("controlsFromSpecs", () => {
  it("starts every control at the default the backend stated", () => {
    expect(controlsFromSpecs(specs)).toEqual({
      mode: "binary", speckle: 4, smoothing: 60, detail: 9.5, colors: 6,
    });
  });
  it("refuses a table missing a control rather than inventing one", () => {
    const short = { ...specs, controls: specs.controls.filter((c) => c.name !== "detail") };
    expect(() => controlsFromSpecs(short)).toThrow(/detail/);
  });
});
```

And replace the empty-sentinel case inside the existing `staleness` describe:

```ts
  it("maps the empty code to the empty state, other codes to error", () => {
    expect(acceptError(2, 2, "empty", "nothing traced — lower the speckle filter or raise detail", { kind: "tracing" }))
      .toEqual({ kind: "empty", message: "nothing traced — lower the speckle filter or raise detail" });
    expect(acceptError(2, 2, "trace", "trace failed: x", { kind: "tracing" }))
      .toEqual({ kind: "error", message: "trace failed: x" });
  });
```

Update the two stale-response assertions in that describe to the new arity:

```ts
    expect(acceptError(1, 2, "trace", "boom", prev)).toBe(prev);
```

- [ ] **Step 2: Run to verify they fail**

```sh
npm --prefix apps/desktop/ui test -- viewmodel
```

Expected: FAIL — `controlsFromSpecs is not exported`, and `acceptError` receiving the wrong argument count.

- [ ] **Step 3: Rewrite the trace wire types**

In `apps/desktop/ui/src/ipc.ts`, replace the trace section (lines 228–245):

```ts
// --- trace wire types ---

export type TraceControlsDto = {
  mode: "binary" | "color";
  speckle: number;
  smoothing: number;
  detail: number;
  colors: number;
};
// Mirrors trace::ControlSpec. No range, default, or step is written on this side — the whole point
// of the command below is that these numbers have one home.
export type ControlSpec = {
  name: "speckle" | "smoothing" | "detail" | "colors";
  label: string;
  help: string;
  min: number;
  max: number;
  step: number;
  default: number;
  colorOnly: boolean;
};
export type TraceControlSpecsDto = {
  controls: ControlSpec[];
  defaultMode: "binary" | "color";
  maxDim: number;
};
export type TraceResultDto = { svg: string; pathCount: number; widthPx: number; heightPx: number; downscaled: boolean };

export async function traceControls(): Promise<TraceControlSpecsDto> {
  return invoke("trace_controls", {});
}
export async function traceImage(args: { path: string; controls: TraceControlsDto }): Promise<TraceResultDto> {
  return invoke("trace_image", args);
}
export async function loadImagePreview(args: { path: string }): Promise<string> {
  return invoke("load_image_preview", args);
}
```

- [ ] **Step 4: Rewrite the viewmodel**

In `apps/desktop/ui/src/trace/viewmodel.ts`, delete the local `TraceControls` type, `defaultControls`,
and `toOptionsDto`. The dialog uses `ipc.TraceControlsDto` directly now.

```ts
// SPDX-License-Identifier: GPL-3.0-or-later
import type { ControlSpec, TraceControlsDto, TraceControlSpecsDto } from "../ipc";

/// Start every control at the default the backend stated, so no default is restated here.
export function controlsFromSpecs(specs: TraceControlSpecsDto): TraceControlsDto {
  const value = (name: ControlSpec["name"]): number => {
    const spec = specs.controls.find((c) => c.name === name);
    // A missing control means the dialog and the tracer disagree about what a trace takes.
    // Rendering a slider from an invented default would hide that; failing says it.
    if (!spec) throw new Error(`trace_controls omitted "${name}"`);
    return spec.default;
  };
  return {
    mode: specs.defaultMode,
    speckle: value("speckle"),
    smoothing: value("smoothing"),
    detail: value("detail"),
    colors: value("colors"),
  };
}

export type PreviewState =
  | { kind: "idle" }
  | { kind: "tracing" }
  | { kind: "ready"; svg: string; pathCount: number; downscaled: boolean }
  | { kind: "empty"; message: string }
  | { kind: "error"; message: string };
```

`acceptResult`, `svgDataUrl`, and `makeDebouncer` are unchanged. `acceptError` gains the code:

```ts
export function acceptError(
  requestId: number, latestId: number, code: string | null, message: string, prev: PreviewState,
): PreviewState {
  if (requestId !== latestId) return prev;
  // The empty state is a distinct rendering, not an error banner. Selecting it by code rather than
  // by matching the message means the wording can change without breaking the branch.
  return code === "empty" ? { kind: "empty", message } : { kind: "error", message };
}
```

- [ ] **Step 5: Run the viewmodel tests**

```sh
npm --prefix apps/desktop/ui test -- viewmodel
```

Expected: PASS.

- [ ] **Step 6: Render the dialog from the table**

In `apps/desktop/ui/src/trace/TraceDialog.tsx`, change the imports and the two pieces of state:

```tsx
import {
  acceptError, acceptResult, controlsFromSpecs, makeDebouncer, svgDataUrl, type PreviewState,
} from "./viewmodel";
```

```tsx
  const [specs, setSpecs] = useState<ipc.TraceControlSpecsDto | null>(null);
  const [controls, setControls] = useState<ipc.TraceControlsDto | null>(null);
```

Add a load effect beside the existing thumbnail effect:

```tsx
  useEffect(() => {
    let ignore = false;
    ipc.traceControls().then(
      (s) => { if (!ignore) { setSpecs(s); setControls(controlsFromSpecs(s)); } },
      (e) => { if (!ignore) setPreview({ kind: "error", message: ipc.ipcErrorMessage(e) }); },
    );
    return () => { ignore = true; };
  }, []);
```

Gate the trace effect on the controls having arrived, and send them unconverted:

```tsx
  useEffect(() => {
    if (controls === null) return;
    const id = ++latestId.current;
    setPreview({ kind: "tracing" });
    debouncer.schedule(() => {
      ipc.traceImage({ path, controls }).then(
        (r) => setPreview((prev) => acceptResult(id, latestId.current, r, prev)),
        (e) => setPreview((prev) =>
          acceptError(id, latestId.current, ipc.ipcErrorCode(e), ipc.ipcErrorMessage(e), prev)),
      );
    });
    return () => debouncer.cancel();
  }, [path, controls, debouncer]);
```

Replace the `slider` helper and the four hardcoded calls (lines 96–103, 145–150) with a map over the
table:

```tsx
        <div style={{ display: "flex", flexDirection: "column", gap: 6, fontSize: 12 }}>
          {specs?.controls.map((s) => {
            const disabled = s.colorOnly && controls?.mode !== "color";
            return (
              <label key={s.name} style={{ display: "flex", alignItems: "center", gap: 8, opacity: disabled ? 0.4 : 1 }}>
                <span style={{ width: 110 }}>{s.label}</span>
                <input type="range" min={s.min} max={s.max} step={s.step} disabled={disabled}
                  value={controls?.[s.name] ?? s.default}
                  onChange={(e) => setControls((c) => c && { ...c, [s.name]: Number(e.target.value) })} />
                <span style={{ width: 32, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                  {controls?.[s.name] ?? s.default}
                </span>
              </label>
            );
          })}
        </div>
```

The mode radios read and write `controls` through the same null guard:

```tsx
            <input type="radio" checked={controls?.mode === "binary"}
              onChange={() => setControls((c) => c && { ...c, mode: "binary" })} /> Binary
```

```tsx
            <input type="radio" checked={controls?.mode === "color"}
              onChange={() => setControls((c) => c && { ...c, mode: "color" })} /> Color
```

Take the two remaining strings from the backend rather than restating them (lines 124, 132):

```tsx
            {preview.kind === "empty" && <span>{preview.message}</span>}
```

```tsx
            {preview.downscaled ? ` — large image reduced to ${specs?.maxDim} px for tracing` : ""}
```

- [ ] **Step 7: Stub the new command in the e2e fake**

In `apps/desktop/ui/e2e/smoke.spec.ts`, add beside `trace_image` (line 143):

```ts
    // A fixture, not a claim: the real table lives in trace::CONTROLS and is what ships. This
    // exists only so the dialog has sliders to render.
    trace_controls: () => ({
      controls: [
        { name: "speckle", label: "Ignore speckles", help: "", min: 0, max: 16, step: 1, default: 4, colorOnly: false },
        { name: "smoothing", label: "Smoothing", help: "", min: 0, max: 180, step: 1, default: 60, colorOnly: false },
        { name: "detail", label: "Detail", help: "", min: 3.5, max: 10, step: 0.5, default: 9.5, colorOnly: false },
        { name: "colors", label: "Colors", help: "", min: 1, max: 8, step: 1, default: 6, colorOnly: true },
      ],
      defaultMode: "binary",
      maxDim: 2048,
    }),
```

- [ ] **Step 8: Build and run every UI check**

```sh
npm --prefix apps/desktop/ui run build
npm --prefix apps/desktop/ui test
npm --prefix apps/desktop/ui run e2e
```

Expected: `tsc` clean, vitest PASS, and both trace e2e tests PASS —
`trace dialog: preview appears and insert adds paths` and
`trace dialog: a failed source thumbnail surfaces instead of blanking`.

- [ ] **Step 9: Confirm the vtracer vocabulary is gone**

```sh
grep -rn "filter_speckle\|corner_threshold\|length_threshold\|color_precision\|filterSpeckle\|cornerThreshold\|lengthThreshold\|colorPrecision\|13\.5" \
  crates/cli apps/desktop/src apps/desktop/ui/src apps/desktop/ui/e2e
```

Expected: no matches. Every hit is a leak this change exists to close.

- [ ] **Step 10: Commit**

```sh
git add apps/desktop/ui
git commit -m "Render the trace dialog from the tracer's own table, not a copy of it

Four ranges, four defaults, the 2048 px ceiling and the empty-result sentence
were all restated in TypeScript. The dialog now asks for them.

toOptionsDto goes with them: the webview sent vtracer's units and inverted
detail on the way out, with 13.5 standing in for 3.5 + 10. trace does the
reflection now, against the bounds it states."
```

---

## Final verification

- [ ] `cargo test --workspace --locked` passes.
- [ ] `npm --prefix apps/desktop/ui test` passes.
- [ ] `npm --prefix apps/desktop/ui run e2e` passes.
- [ ] `git status` is clean — in particular `apps/desktop/ui/dist/` was rebuilt and committed.
- [ ] `cuthulhu trace --help` shows each control's range, sourced from `trace::CONTROLS`.
- [ ] The grep in Task 3 Step 9 returns nothing.
