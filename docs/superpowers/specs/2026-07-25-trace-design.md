# SP5 Trace — bitmap-to-vector design spec

Date: 2026-07-25
Status: approved (brainstorming complete)

## Purpose

Turn a raster image (logo, silhouette, lettering) into cuttable vector paths inside the editor. Fifth sub-project of the master design (`2026-07-21-cuthulhu-design.md` §Build order): "Trace — vtracer integration with cleanup controls."

Decisions made during brainstorming:

- **Engine: vtracer** (MIT, GPL-compatible, sanctioned by the master spec). Cloud services (e.g. vectorizer.ai) rejected: the master spec rules out cloud accounts, and a paid per-image API contradicts the free/offline/no-lock-in premise.
- **One-shot conversion.** The source bitmap does not live in the document. No new node kind, no project-format change. Print & cut (SP6) may add an Image node later if it needs one.
- **Both trace modes**: binary (single-color silhouette) and color (one path per color cluster — feeds SP4 cut-by-color directly).
- **Debounced live preview** in the trace dialog.
- **Curated 4 cleanup controls**; remaining vtracer knobs pinned to defaults.
- **CLI subcommand** `cuthulhu trace` (community-facing artifact, headless test surface).

## Architecture (Approach A: emit SVG, reuse the import pipeline)

The trace crate produces an SVG string. Three consumers share it:

1. **Dialog preview** renders it directly via `<img src="data:image/svg+xml,…">` — the browser rasterizes; no new canvas code.
2. **Insert** feeds it through the existing `import_svg(bytes, parent) -> (Delta, skipped)` — path parsing, style mapping, undo, and warnings are all reused. No new document/delta code.
3. **CLI** writes it to disk.

Rejected alternatives: structured output with direct document insertion (duplicates serialization, new delta command, no reuse of import warnings); vtracer-as-wasm in the UI (violates "Rust core fully testable headless; UI reaches the engine only through IPC").

### `crates/trace` (new)

```rust
pub enum TraceMode { Binary, Color }

pub struct TraceOptions {
    pub mode: TraceMode,          // default Binary
    pub filter_speckle: u8,       // 0–16 px, default 4   ("Ignore speckles")
    pub corner_threshold: u8,     // 0–180°, default 60   ("Smoothing")
    pub length_threshold: f64,    // 3.5–10.0, default 4.0 ("Detail")
    pub color_precision: u8,      // 1–8 bits, default 6  ("Colors", color mode only)
}

pub struct TraceResult {
    pub svg: String,
    pub path_count: usize,
    pub width_px: u32,            // post-downscale dimensions
    pub height_px: u32,
    pub downscaled: bool,         // true when the 2048 px cap was applied
}

pub enum TraceError { InvalidOption(String), Decode(String), Trace(String), EmptyResult }

pub fn trace(image_bytes: &[u8], opts: &TraceOptions) -> Result<TraceResult, TraceError>
```

Internals:

- `image` crate decodes PNG/JPEG/GIF/BMP → RGBA.
- Images over **2048 px max-dimension are downscaled** to fit (aspect preserved) before tracing. Bounds trace time; not an error.
- Dimensions are read from the header **before** decoding, and an image whose raw RGBA buffer would exceed **512 MiB** is rejected as `Decode`. The downscale cap bounds what vtracer sees, not what the decoder allocates, so without this a compressed bomb (a 75 KB 20000×20000 PNG expands to 1.6 GB) exhausts memory first.
- vtracer's clustering layer panics on some valid high-frequency inputs (a 512×512 one-pixel checkerboard overflows in `visioncortex`). The conversion runs inside `catch_unwind` and a panic surfaces as `TraceError::Trace`, so neither the CLI nor the Tauri command aborts.
- vtracer `convert(ColorImage, Config)`; non-exposed knobs pinned: `mode: Spline`, `hierarchical: Stacked`, `layer_difference`, `splice_threshold`, `max_iterations`, `path_precision` at vtracer defaults.
- Options validated to the ranges above; out-of-range → `TraceError::InvalidOption` naming the field and range.
- Zero paths traced → `TraceError::EmptyResult`.

### Units

vtracer output is in image pixels. `fileio::svg_to_paths` already converts px→mm at 96 DPI (`PX_TO_MM = 25.4/96`), so inserted geometry is sized automatically (e.g. a 960 px logo lands 254 mm wide). Users rescale on canvas with existing transforms. No size control in the dialog.

### IPC

One new async command:

```rust
#[tauri::command(async)]
trace_image(path: PathBuf, opts: TraceOptions) -> Result<TraceResult, String>
```

Rust reads the file, so the *trace* path never sends bitmap bytes across IPC — only the SVG string returns. Insert is the **existing** `import_svg` command called from the UI with that SVG string as bytes, parent = document root.

A second command backs the source-image thumbnail:

```rust
#[tauri::command(async)]
load_image_preview(path: PathBuf) -> Result<String, String>
```

It returns a `data:<mime>;base64,…` URL, so the source bitmap **does** cross IPC, base64-encoded (~1.37× the file size). This is a deliberate departure from `convertFileSrc`, which would require asset-protocol scope configuration for arbitrary user-picked paths and is awkward to mock in the e2e suite. The cost is that a large source image is briefly held as a base64 string in the webview — though `MAX_DIM` bounds that too, since `load_image_preview` re-encodes through the same decode-and-downscale path as tracing, capping the payload at 2048 px on the long edge rather than at the source file's size.

Both commands read any path the webview supplies, so both are constrained to return only image-derived data:

- `trace_image` returns SVG, and a path that does not decode as an image errors.
- `load_image_preview` re-encodes through the decoder and returns a PNG, rather than the file's original bytes.

That distinction is load-bearing. Returning raw bytes would have made the thumbnail command a general "read any file, hand back its contents" primitive, which the app did not previously have. `load_project` is **not** an equivalent primitive despite also taking a caller-supplied path: it opens the file as a zip and requires a `manifest.json` member, so a non-project path fails without disclosing anything. An earlier draft of this spec claimed otherwise and was wrong.

Both commands also refuse files over 256 MiB before reading them, since the decoder's ceiling only applies once the bytes are already in memory.

Neither command trusts the caller's path on its own. The native picker lives in Rust as `pick_image`, which records each canonicalized selection in per-session state; `trace_image` and `load_image_preview` both refuse a path that is not in that set. Selection by the user is therefore what grants access, and the renderer cannot authorize a path — only ask that one be authorized by the user choosing it. Paths are canonicalized on both sides of the check so an authorized file cannot be re-reached under a different spelling via `..` segments or a symlink.

This is why the picker is not the `tauri-plugin-dialog` call it was in the first draft: a picker running in the webview proves nothing to the backend.

**Accepted residual:** authorization records a pathname, not a file identity, so the window is wider than the gap between resolving a path and opening it — replacing the picked pathname at any point after the user selects it yields a file this app will then trace. This is deliberately not defended against. (The read itself is no longer a second resolution: `read_image_file` opens the path once, checks the size with `fstat` on that handle, and bounds the read with `take`. The size check still refuses an oversized file cheaply, but it is no longer what enforces the ceiling — a file that grows after being measured can no longer be read past the limit.) Exploiting any of it requires a separate local process with write access to the user's files, which can already read those files without involving this app, so no privilege boundary is crossed. (The renderer is not entirely without filesystem reach — `save_project` renames a temp file over a caller-supplied path — but it writes a project zip, so it cannot plant an image to be disclosed.) The atomic alternative, pinning the `(dev, ino)` recorded at pick time and verifying it against the opened handle, would reject an image the user legitimately re-saved between picking and tracing, since editors commonly write-then-rename and change the inode. That is a real failure on an ordinary path traded against an attack that gains nothing.

Rendering these data URLs requires `img-src 'self' data:` in the Tauri CSP (`apps/desktop/tauri.conf.json`); under the default `default-src 'self'` both previews are blocked in a packaged build.

### UI: `TraceDialog.tsx` + `trace/viewmodel.ts`

CutDialog pattern (dialog + pure viewmodel + tests).

- Entry: TopBar "Trace…" button → native file picker (`tauri-plugin-dialog`, image-extension filters).
- Layout: source bitmap (`<img>` via `load_image_preview`'s base64 data URL) beside traced preview (SVG data URL); mode toggle (Binary/Color); sliders: Ignore speckles, Smoothing, Detail, and Colors (enabled in color mode only). The Detail slider displays an inverted scale of `length_threshold` (slider up = more detail = lower threshold), so the empty-state hint "raise detail" is directionally correct.
- Slider/mode changes re-trace after **300 ms debounce**; a monotonic request id discards stale responses (SP4 job-id pattern).
- Insert button → `import_svg` → dialog closes. Path count is shown in the dialog before inserting; import-side skips surface through the existing StatusBar error channel ("Inserted with M element(s) skipped"). Undo removes the whole insertion (single Delta).

### CLI

```
cuthulhu trace in.png -o out.svg [--mode binary|color] [--speckle 4] [--smoothing 60] [--detail 4.0] [--colors 6]
```

Thin wrapper over `trace::trace`; writes `TraceResult.svg` to the output path.

## Error handling

No silent failures; every error path surfaces.

| Failure | Surface |
|---|---|
| `Decode` (unreadable/unsupported file) | Dialog error banner; controls stay usable; user may pick another file |
| `Trace` (vtracer internal) | Same banner, message passed through |
| `EmptyResult` | Not a banner — preview area hint: "Nothing traced — lower speckle filter or raise detail" |
| `InvalidOption` | Unreachable from UI (sliders clamp); CLI prints field + valid range, exit ≠ 0 |
| `EmptyResult` (CLI) | "Nothing traced — adjust --speckle/--detail", exit ≠ 0, no file written |
| Downscale applied | Informational note in dialog (driven by `TraceResult.downscaled`): "Large image reduced to 2048 px for tracing" |
| Insert-side skips | Existing `import_svg` skipped-warnings channel → StatusBar |

## Testing

- **`crates/trace` unit tests** — synthetic in-code bitmaps (black square on white; two-color blocks): path count ≥ 1; binary mode yields a single style; color mode yields ≥ 2 fills; determinism (same input + opts → identical SVG string); downscale triggers above cap and preserves aspect; option validation rejects out-of-range; decode error on garbage bytes.
- **Round-trip test** (the load-bearing one for Approach A): `trace()` output → `fileio::svg_to_paths` parses with 0 skipped. Pins the "vtracer SVG is always consumable by our importer" contract at the crate seam.
- **CLI integration test** — `trace` subcommand on a fixture PNG: SVG written, exit 0; bad flag → nonzero exit.
- **UI viewmodel tests** — debounce collapses rapid changes to one call; stale (lower request id) response discarded; option mapping to the IPC payload; empty-result hint state.
- **e2e smoke** — extend the mocked `__TAURI_INTERNALS__` with `trace_image`; open dialog → preview appears → Insert adds nodes. Existing smoke assertions unchanged.

## Out of scope (SP5)

- Image node in the document / re-trace after insert.
- Cloud tracing backends (a `Tracer` trait indirection was considered and dropped — YAGNI).
- Output-size control in the dialog (canvas transforms cover it).
- Exposing vtracer's remaining knobs (advanced mode) — add on demand.
