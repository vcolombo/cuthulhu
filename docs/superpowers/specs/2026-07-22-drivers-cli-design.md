# Sub-project 2: Drivers + CLI — design

The driver layer and command-line tool that make both reference machines cut a
square from an SVG. Second sub-project, after the protocol spike (SP1). Governed
by the master design spec (`2026-07-21-cuthulhu-design.md`); this document specs
SP2 only.

**Goal:** `cuthulhu cut square.svg --device <cameo5|puma>` drives the real
machine to cut the square — proving the whole premise (SVG in, correct device
bytes out, machine responds) before any UI exists. This is the riskiest
unknown, retired second.

## Decisions (settled in brainstorming)

- **Geometry scope: cut-path only.** SP2 builds the path model, curve flattening, affine transforms, and SVG parsing. **Booleans and text-to-path are deferred to SP3** (editor features; the cut pipeline does not need them).
- **Transport: write-only + mock.** `driver-core`'s `Transport` writes bytes; a `MockTransport` records them for tests. Status read-back (Silhouette ENQ, wait-for-ready) defers to SP4.
- **USB via nusb** (pure-Rust, no libusb system dependency); **serial via the serialport crate.**
- **`--dry-run` is the hardware-free spine** — the full pipeline emits bytes with no device attached; CI and pre-bench verification run on it.

## Architecture — the vertical slice

The one flow SP2 must nail, SVG file to machine:

```
square.svg
  → fileio::svg_to_paths     usvg parse → Vec<Path> (mm)
  → geometry::flatten        Bézier → polylines (tolerance)
  → driver-core::Job         { polylines(mm), settings }
  → Driver::encode(job)      → Vec<u8> (GPGL or HPGL device bytes)
  → Transport::write(bytes)  → machine cuts
```

Crates (the workspace SP2 bootstraps under `crates/` + `apps/` is SP3):

| Crate | SP2 responsibility |
|-------|--------------------|
| `geometry` | `Path`, `Affine`, `Rect`, `Point`/`Polyline`, `flatten`, `from_svg`/`to_svg`. Pure mm. No booleans/text. |
| `fileio` | `svg_to_paths(bytes)` via usvg. SVG import only. |
| `driver-core` | `Transport` + `Driver` traits, `Job`/`Settings`, `MachineProfile`, `MockTransport`, error types. Trait/data only — no dependency on the concrete driver crates. |
| `driver-silhouette` | `SilhouetteDriver` (GPGL encode) + `UsbTransport` (nusb, `0x3844:0x0001`). |
| `driver-hpgl` | `HpglDriver` (HPGL encode) + `SerialTransport` (serialport). |
| `cli` | `cuthulhu cut …`, `cuthulhu list-devices`; owns the device registry that wires ids to concrete drivers + transports. |

## `driver-core`

Geometry hand-off types live in `geometry` and are consumed here:
```rust
pub struct Point { pub x: f64, pub y: f64 }   // mm
pub type Polyline = Vec<Point>;                // flatten() output
```

```rust
pub struct Job { pub polylines: Vec<Polyline>, pub settings: Settings }  // polylines in mm
pub struct Settings { pub speed: Option<u32>, pub force: Option<u32>, pub passes: u32 }
// Default: speed None, force None, passes 1.

pub struct MachineProfile { pub id: String, pub name: String, pub width_mm: f64, pub height_mm: f64 }

pub trait Driver {
    fn encode(&self, job: &Job) -> Result<Vec<u8>, DriverError>;
    fn profile(&self) -> &MachineProfile;
}
pub trait Transport { fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError>; }

#[derive(Default)]
pub struct MockTransport { pub written: Vec<u8> }   // records for tests

pub enum DriverError { UnsupportedGeometry, Encode(String) }
pub enum TransportError { NotFound, Io(String) }
```

`Settings.speed`/`force` are `Option` deliberately: the HPGL driver **ignores** them (GCC force/speed are set on the machine panel in V1, per `gcc-hpgl.md`); the Silhouette driver **applies** them (`!`/`FX`). This asymmetry is documented, not accidental. `driver-core` has no dependency on the concrete driver crates — the CLI is the only place that knows all drivers, keeping the graph acyclic. `MachineProfile` is the authoritative source that SP3 stubbed.

## Drivers

Both encoders mirror the committed protocol docs. The SP1 senders (`send_raw.py`, `hpgl.py`) are the golden byte-stream references for the encode tests.

### `driver-silhouette` (GPGL / USB)

Encode per `silhouette-cameo5.md` — 20 units/mm, `(y,x)` order, ETX(`0x03`) after each command:
```
ESC EOT                        init (0x1b 0x04)
J<tool>                        tool select (default 1)
!<speed>,<tool>                only if settings.speed is Some
FX<force>,<tool>               only if settings.force is Some
per polyline:
  M<y_su>,<x_su>               move (pen up) to first point
  D<y_su>,<x_su>  (× rest)     draw (pen down) through remaining points
SO0
FN0
```
`su = round(mm * 20.0)`. `passes > 1` repeats the `M/D` block.

`UsbTransport` (nusb): find `0x3844:0x0001`, detach kernel driver if active, claim interface 0, bulk-write endpoint `0x01`.

### `driver-hpgl` (HPGL / serial)

Encode per `gcc-hpgl.md` — 1016 units/inch, `(x,y)`, `;`-terminated:
```
IN;
per polyline:
  PU<x_u>,<y_u>;               pen up to first point
  PD<x_u>,<y_u>;  (× rest)     pen down through remaining points
PU;                            final pen up
```
`u = round(mm / 25.4 * 1016)`. Ignores `speed`/`force` (panel-set in V1). `passes > 1` repeats the `PU/PD` block.

`SerialTransport` (serialport): open the named port at `9600 8N1` (configurable), write the stream.

### Coordinate reality
Two grids, each owned by its encoder; `driver-core` never sees device units:
- Silhouette: `(y,x)`, 20/mm.
- HPGL: `(x,y)`, 40/mm (1016/in).

## `geometry` (cut-path only)

- Types: `Point`, `Polyline`, `Path` (subpaths of line/cubic segments, mm), `Affine`, `Rect`.
- `Affine`: `identity/translate/then/inverse/apply`; **derives `Serialize/Deserialize/PartialEq`** (SP3 needs this).
- `Path`: `flatten(&self, tol_mm) -> Vec<Polyline>` (adaptive subdivision, default 0.1 mm), `transformed(&Affine)`, `bounds() -> Rect`, `from_svg`/`to_svg`.
- **No mm↔device helpers** — device units are per-machine and live in each encoder. Geometry is pure mm.
- **No booleans, no text-to-path** (SP3 adds them to this crate).

## `fileio` (SVG import only)

- `svg_to_paths(bytes) -> Result<SvgImport, IoError>` via **usvg**: walk the resolved tree, convert path geometry to `geometry::Path`, carry a `StyleHint { stroke, fill }`. SVG px → mm at 96 dpi (`1px = 25.4/96 mm`); this assumption is documented in code.
- `SvgImport { paths: Vec<(Path, StyleHint)>, skipped: Vec<String> }` — unsupported elements (raster images, live text) are reported in `skipped`, never silently dropped.
- No project save/load (SP3).

## CLI

```
cuthulhu cut <file.svg> --device <id> [--dry-run] [--speed N] [--force N] [--port P] [--baud B]
cuthulhu list-devices
```

- **Device registry (in `cli`):** `cameo5` → `SilhouetteDriver` + `UsbTransport`; `puma` → `HpglDriver` + `SerialTransport(port, baud)`.
- **Flow:** read SVG → `svg_to_paths` → `flatten` each path → build `Job` → `driver.encode` → `--dry-run` prints the byte stream (hex + ASCII), else `transport.write`.
- `list-devices` prints known ids + their `MachineProfile`.

## Error handling

No silent failures (master-spec ethos). Every stage returns a typed `Result`; the CLI maps errors to a clear message and a non-zero exit. A malformed SVG, an unfound device, or unsupported geometry each report explicitly. `svg_to_paths` additionally surfaces `skipped` elements even on success.

## Testing (all headless, CI-safe)

- `geometry`: flatten golden (cubic → polyline within tolerance), transform, bounds.
- `fileio`: `svg_to_paths` golden (rect SVG → one path, known mm coords) + skipped report.
- `driver-silhouette`: **encode golden** — a square `Job` → the exact GPGL stream from `silhouette-cameo5.md`, cross-checked against `send_raw.py`.
- `driver-hpgl`: **encode golden** — square → `IN;PU0,0;PD0,800;…;PU;`, cross-checked against `hpgl.py`.
- `MockTransport`: asserts the driver writes exactly the encoded bytes.
- `cli`: integration — `cut square.svg --device cameo5 --dry-run` stdout == golden; same for `puma`.
- nusb/serialport real paths run only on the manual hardware step, never in CI.
- CI gains a `cargo test` job (the SP1 CI workflow reserved this slot).

## Definition of done

- `cuthulhu cut square.svg --device cameo5 --dry-run` emits the exact GPGL stream (matches `silhouette-cameo5.md`).
- `cuthulhu cut square.svg --device puma --dry-run --port …` emits the exact HPGL stream (matches `gcc-hpgl.md`).
- **Physical gate (manual, hardware):** each machine cuts the 20 mm square — the bench step SP1 already teed up.
- Everything else green headless: golden tests + dry-run in CI.

## Dependencies

- SP1's protocol notes (`silhouette-cameo5.md`, `gcc-hpgl.md`) and senders (byte-stream references). No hardware needed until the physical gate.
- Establishes the Rust workspace + `geometry`/`fileio`/`driver-core` that **SP3 builds on**.

## Out of scope (SP2)

- Booleans, text-to-path (SP3 extends `geometry`).
- Project save/load (SP3 extends `fileio`).
- Device status read-back / wait-for-ready (SP4).
- Cut settings beyond speed/force/passes; material presets (SP4).
- Any UI (SP3).
- PDF/DXF import.
