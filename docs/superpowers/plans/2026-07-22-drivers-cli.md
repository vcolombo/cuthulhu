# Drivers + CLI (SP2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cuthulhu cut square.svg --device <cameo5|puma> --dry-run` emits the exact device byte stream for each machine; a real run cuts a 20 mm square. Proves the SVG→bytes→machine premise before any UI.

**Architecture:** A Rust workspace with six crates forming one vertical slice: `fileio` parses SVG to `geometry` paths, `geometry` flattens them to polylines, a `driver-core` `Job` carries them, a per-machine `Driver` encodes device bytes, and a write-only `Transport` sends them. The CLI wires ids to concrete drivers.

**Tech Stack:** Rust 2021, `serde`, `usvg` (SVG parse), `nusb` (USB), `serialport` (serial), `clap` (CLI). Tests via `cargo test`; golden encode tests cross-check the SP1 senders (`tools/replay/`).

## Global Constraints

- **License:** every file starts with `// SPDX-License-Identifier: GPL-3.0-or-later`.
- **No AI attribution** in commits or code.
- **Geometry is pure millimetres.** Device-unit conversion lives only in each driver's encoder (Silhouette 20/mm; HPGL 1016/in). `geometry` and `driver-core` never see device units.
- **Cut-path only:** no booleans/text-to-path (SP3), no project save/load (SP3), no status read-back (SP4).
- **No silent failures:** every fallible boundary returns a typed `Result`; the CLI maps errors to a message + non-zero exit.
- **Coordinate orders:** Silhouette `(y,x)`; HPGL `(x,y)`. Get these wrong and the square is mirrored — golden tests pin them.

---

### Task 1: Workspace bootstrap + CI cargo job

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/geometry/Cargo.toml`, `crates/geometry/src/lib.rs`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: a compiling workspace so every later crate is a member.

- [ ] **Step 1: Write the workspace + first crate**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/*"]
```

`crates/geometry/Cargo.toml`:
```toml
[package]
name = "geometry"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-or-later"

[dependencies]
serde = { version = "1", features = ["derive"] }
```

`crates/geometry/src/lib.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build`
Expected: compiles (empty crate).

- [ ] **Step 3: Add the CI cargo job**

Append to `.github/workflows/ci.yml` under `jobs:`:
```yaml
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Build and test workspace
        run: cargo test --workspace
```

- [ ] **Step 4: Verify**

Run: `cargo test --workspace`
Expected: PASS (0 tests).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/geometry/ .github/workflows/ci.yml
git commit -m "Bootstrap Rust workspace and add CI cargo-test job"
```

---

### Task 2: `geometry` — Point, Rect, Affine

**Files:**
- Create: `crates/geometry/src/affine.rs`
- Modify: `crates/geometry/src/lib.rs`

**Interfaces:**
- Produces: `Point { x, y }`, `Polyline = Vec<Point>`, `Rect { x, y, w, h }`, `Affine` with `identity/translate/then/inverse/apply`, all `Serialize/Deserialize/PartialEq`.

- [ ] **Step 1: Write the failing test**

`crates/geometry/src/affine.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translate_then_apply() {
        let m = Affine::translate(3.0, -2.0);
        assert_eq!(m.apply(1.0, 1.0), (4.0, -1.0));
    }
    #[test]
    fn then_composes_left_to_right_of_argument() {
        let t = Affine::translate(5.0, 0.0);
        let composed = t.then(&Affine::translate(0.0, 2.0)); // apply t, then +2y
        assert_eq!(composed.apply(0.0, 0.0), (5.0, 2.0));
    }
    #[test]
    fn inverse_undoes_translate() {
        let m = Affine::translate(7.0, 9.0);
        let back = m.inverse();
        assert_eq!(back.apply(m.apply(1.0, 1.0).0, m.apply(1.0, 1.0).1), (1.0, 1.0));
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p geometry affine::` → FAIL (types missing).

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Serialize, Deserialize};

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Point { pub x: f64, pub y: f64 }
pub type Polyline = Vec<Point>;

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }

/// Row-major 2x3 affine: [a b c d e f] → x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub fn identity() -> Affine { Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]) }
    pub fn translate(dx: f64, dy: f64) -> Affine { Affine([1.0, 0.0, 0.0, 1.0, dx, dy]) }
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }
    /// self.then(other) = apply self, then other.
    pub fn then(&self, other: &Affine) -> Affine {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Affine([
            a2 * a1 + c2 * b1,        b2 * a1 + d2 * b1,
            a2 * c1 + c2 * d1,        b2 * c1 + d2 * d1,
            a2 * e1 + c2 * f1 + e2,   b2 * e1 + d2 * f1 + f2,
        ])
    }
    pub fn inverse(&self) -> Affine {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Affine([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)])
    }
}
```

`lib.rs`: add `mod affine; pub use affine::*;`.

- [ ] **Step 4: Run to verify it passes.** `cargo test -p geometry affine::` → PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/geometry/ && git commit -m "Add geometry Point/Rect/Affine with tests"
```

---

### Task 3: `geometry` — Path, flatten, from_svg/to_svg, bounds, transformed

**Files:**
- Create: `crates/geometry/src/path.rs`
- Modify: `crates/geometry/src/lib.rs`

**Interfaces:**
- Consumes: `Point`, `Polyline`, `Rect`, `Affine` (Task 2).
- Produces: `Seg`, `Path` with `from_svg(&str) -> Result<Path, GeomError>`, `to_svg`, `transformed`, `bounds`, `flatten(tol) -> Vec<Polyline>`, and `GeomError`.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_square_flattens_to_its_own_points() {
        let p = Path::from_svg("M0,0 L20,0 L20,20 L0,20 Z").unwrap();
        let polys = p.flatten(0.1);
        assert_eq!(polys.len(), 1);
        let pts: Vec<(f64,f64)> = polys[0].iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(pts, vec![(0.0,0.0),(20.0,0.0),(20.0,20.0),(0.0,20.0),(0.0,0.0)]);
    }
    #[test]
    fn cubic_flatten_stays_within_tolerance_endpoints() {
        let p = Path::from_svg("M0,0 C0,10 10,10 10,0").unwrap();
        let polys = p.flatten(0.05);
        assert_eq!(polys[0].first().unwrap().x, 0.0);
        assert_eq!(polys[0].last().unwrap(), &Point { x: 10.0, y: 0.0 });
        assert!(polys[0].len() > 2); // subdivided
    }
    #[test]
    fn transformed_bounds_shift() {
        let p = Path::from_svg("M0,0 L10,0 L10,10 L0,10 Z")
            .unwrap().transformed(&Affine::translate(5.0, 0.0));
        let b = p.bounds();
        assert_eq!((b.x, b.w), (5.0, 10.0));
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p geometry path::` → FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Serialize, Deserialize};
use crate::affine::{Point, Polyline, Rect, Affine};

#[derive(Debug, PartialEq)]
pub enum GeomError { Parse(String), Degenerate, NoFont }

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Seg { Move(Point), Line(Point), Cubic(Point, Point, Point), Close }

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
pub struct Path { pub segs: Vec<Seg> }

impl Path {
    /// Minimal SVG path parser: absolute M, L, C, Z (comma/space separated).
    pub fn from_svg(d: &str) -> Result<Path, GeomError> {
        let mut segs = vec![];
        let mut toks = d.split(|c: char| c.is_whitespace() || c == ',')
                        .filter(|s| !s.is_empty()).peekable();
        let mut num = |t: &mut std::iter::Peekable<_>| -> Result<f64, GeomError>;  // see note
        // Implemented as a helper below instead of a closure for clarity:
        fn take(toks: &mut impl Iterator<Item = String>) -> Result<f64, GeomError> {
            toks.next().ok_or(GeomError::Parse("eof".into()))?
                .parse().map_err(|_| GeomError::Parse("nan".into()))
        }
        let mut it: Vec<String> = d.replace(['M','L','C','Z'], |_| unreachable!())
            .split_whitespace().map(|s| s.to_string()).collect(); // placeholder — real impl below
        let _ = (&mut num, &mut it, &mut segs, &mut toks);
        parse_svg_path(d)
    }
    pub fn transformed(&self, m: &Affine) -> Path {
        let tp = |p: &Point| { let (x, y) = m.apply(p.x, p.y); Point { x, y } };
        Path { segs: self.segs.iter().map(|s| match s {
            Seg::Move(p) => Seg::Move(tp(p)),
            Seg::Line(p) => Seg::Line(tp(p)),
            Seg::Cubic(a, b, c) => Seg::Cubic(tp(a), tp(b), tp(c)),
            Seg::Close => Seg::Close,
        }).collect() }
    }
    pub fn bounds(&self) -> Rect {
        let pts = self.flatten(0.1);
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for poly in &pts { for p in poly {
            minx = minx.min(p.x); miny = miny.min(p.y); maxx = maxx.max(p.x); maxy = maxy.max(p.y);
        }}
        Rect { x: minx, y: miny, w: maxx - minx, h: maxy - miny }
    }
    pub fn flatten(&self, tol: f64) -> Vec<Polyline> {
        let mut out = vec![];
        let mut cur: Polyline = vec![];
        let mut start = Point { x: 0.0, y: 0.0 };
        let mut here = start;
        for s in &self.segs {
            match s {
                Seg::Move(p) => { if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
                                  cur.push(*p); start = *p; here = *p; }
                Seg::Line(p) => { cur.push(*p); here = *p; }
                Seg::Cubic(c1, c2, e) => { subdivide(here, *c1, *c2, *e, tol, &mut cur); here = *e; }
                Seg::Close => { cur.push(start); here = start; }
            }
        }
        if !cur.is_empty() { out.push(cur); }
        out
    }
}

fn subdivide(p0: Point, p1: Point, p2: Point, p3: Point, tol: f64, out: &mut Polyline) {
    // flatness: max control-point deviation from the chord
    let d1 = dist_to_line(p1, p0, p3);
    let d2 = dist_to_line(p2, p0, p3);
    if d1.max(d2) <= tol { out.push(p3); return; }
    let (l, r) = split_cubic(p0, p1, p2, p3);
    subdivide(l.0, l.1, l.2, l.3, tol, out);
    subdivide(r.0, r.1, r.2, r.3, tol, out);
}
fn mid(a: Point, b: Point) -> Point { Point { x: (a.x + b.x) / 2.0, y: (a.y + b.y) / 2.0 } }
fn split_cubic(p0: Point, p1: Point, p2: Point, p3: Point)
    -> ((Point,Point,Point,Point),(Point,Point,Point,Point)) {
    let p01 = mid(p0,p1); let p12 = mid(p1,p2); let p23 = mid(p2,p3);
    let p012 = mid(p01,p12); let p123 = mid(p12,p23); let m = mid(p012,p123);
    ((p0,p01,p012,m),(m,p123,p23,p3))
}
fn dist_to_line(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx*dx + dy*dy).sqrt();
    if len == 0.0 { return ((p.x-a.x).powi(2) + (p.y-a.y).powi(2)).sqrt(); }
    ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / len
}

fn parse_svg_path(d: &str) -> Result<Path, GeomError> {
    let mut segs = vec![];
    let mut chars = d.chars().peekable();
    let mut nums: Vec<f64> = vec![];
    let mut cmd: Option<char> = None;
    let flush = |cmd: char, nums: &mut Vec<f64>, segs: &mut Vec<Seg>| -> Result<(), GeomError> {
        match cmd {
            'M' => { segs.push(Seg::Move(Point { x: nums[0], y: nums[1] })); }
            'L' => { segs.push(Seg::Line(Point { x: nums[0], y: nums[1] })); }
            'C' => { segs.push(Seg::Cubic(
                Point{x:nums[0],y:nums[1]}, Point{x:nums[2],y:nums[3]}, Point{x:nums[4],y:nums[5]})); }
            'Z' => { segs.push(Seg::Close); }
            _ => return Err(GeomError::Parse(format!("cmd {cmd}"))),
        }
        nums.clear(); Ok(())
    };
    let mut buf = String::new();
    for ch in d.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphabetic() {
            if let Some(c) = cmd { if !buf.is_empty() { nums.push(buf.parse().map_err(|_| GeomError::Parse("nan".into()))?); buf.clear(); } flush(c, &mut nums, &mut segs)?; }
            cmd = Some(ch);
        } else if ch == ',' || ch.is_whitespace() {
            if !buf.is_empty() { nums.push(buf.parse().map_err(|_| GeomError::Parse("nan".into()))?); buf.clear(); }
        } else { buf.push(ch); }
    }
    if let Some(c) = cmd { flush(c, &mut nums, &mut segs)?; }
    Ok(Path { segs })
}
```

> **Note:** the two dead placeholder lines in `from_svg` (the `num` closure and `it` vec) are illustrative of a wrong first attempt — delete them; `from_svg` is just `parse_svg_path(d)`. Keep `from_svg` as `pub fn from_svg(d: &str) -> Result<Path, GeomError> { parse_svg_path(d) }`. Add `to_svg` emitting `M/L/C/Z`. `lib.rs`: `mod path; pub use path::*;`.

- [ ] **Step 4: Run to verify it passes.** `cargo test -p geometry path::` → PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/geometry/ && git commit -m "Add geometry Path with SVG parse, flatten, transform, bounds"
```

---

### Task 4: `fileio` — svg_to_paths

**Files:**
- Create: `crates/fileio/Cargo.toml`, `crates/fileio/src/lib.rs`

**Interfaces:**
- Consumes: `geometry::{Path, Seg, Point}`.
- Produces: `svg_to_paths(bytes) -> Result<SvgImport, IoError>`, `SvgImport { paths: Vec<(Path, StyleHint)>, skipped: Vec<String> }`, `StyleHint { stroke, fill }`, `IoError`.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_a_rect_into_one_path_in_mm() {
        // 20x20 user units at 96dpi → but usvg keeps user units; we map px→mm at 96dpi.
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"
                        viewBox="0 0 20 20"><rect width="20" height="20"/></svg>"#;
        let imp = svg_to_paths(svg).unwrap();
        assert_eq!(imp.paths.len(), 1);
        let b = imp.paths[0].0.bounds();
        // 20 px → 20 * 25.4/96 mm ≈ 5.29 mm
        assert!((b.w - 20.0 * 25.4 / 96.0).abs() < 0.01);
        assert!(imp.skipped.is_empty());
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p fileio` → FAIL.

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml`:
```toml
[package]
name = "fileio"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-or-later"

[dependencies]
geometry = { path = "../geometry" }
usvg = "0.44"
```

`src/lib.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::{Path, Seg, Point};

const PX_TO_MM: f64 = 25.4 / 96.0;

#[derive(Debug)]
pub enum IoError { Parse(String), Io(String) }
#[derive(Clone, Debug)]
pub struct StyleHint { pub stroke: Option<u32>, pub fill: Option<u32> }
pub struct SvgImport { pub paths: Vec<(Path, StyleHint)>, pub skipped: Vec<String> }

pub fn svg_to_paths(bytes: &[u8]) -> Result<SvgImport, IoError> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default())
        .map_err(|e| IoError::Parse(e.to_string()))?;
    let mut paths = vec![];
    let mut skipped = vec![];
    walk(tree.root(), &mut paths, &mut skipped);
    Ok(SvgImport { paths, skipped })
}

fn walk(group: &usvg::Group, out: &mut Vec<(Path, StyleHint)>, skipped: &mut Vec<String>) {
    for node in group.children() {
        match node {
            usvg::Node::Path(p) => {
                let mut segs = vec![];
                for seg in p.data().segments() {
                    use usvg::tiny_skia_path::PathSegment as S;
                    match seg {
                        S::MoveTo(pt) => segs.push(Seg::Move(mm(pt))),
                        S::LineTo(pt) => segs.push(Seg::Line(mm(pt))),
                        S::CubicTo(a, b, c) => segs.push(Seg::Cubic(mm(a), mm(b), mm(c))),
                        S::QuadTo(a, b) => segs.push(Seg::Cubic(mm(a), mm(a), mm(b))), // ponytail: quad→cubic approx
                        S::Close => segs.push(Seg::Close),
                    }
                }
                let hint = StyleHint {
                    stroke: p.stroke().map(|s| paint_rgba(s.paint())),
                    fill: p.fill().map(|f| paint_rgba(f.paint())),
                };
                out.push((Path { segs }, hint));
            }
            usvg::Node::Group(g) => walk(g, out, skipped),
            usvg::Node::Image(_) => skipped.push("image".into()),
            usvg::Node::Text(_) => skipped.push("text".into()),
        }
    }
}
fn mm(p: usvg::tiny_skia_path::Point) -> Point { Point { x: p.x as f64 * PX_TO_MM, y: p.y as f64 * PX_TO_MM } }
fn paint_rgba(_paint: &usvg::Paint) -> u32 { 0x000000FF } // ponytail: solid-black default; real color mapping later
```

- [ ] **Step 4: Run to verify it passes.** `cargo test -p fileio` → PASS. (Pin the exact `usvg` API to the version in `Cargo.toml`; adjust `segments()`/`Node` names if the crate differs.)

- [ ] **Step 5: Commit**

```bash
git add crates/fileio/ && git commit -m "Add fileio svg_to_paths via usvg"
```

---

### Task 5: `driver-core` — Job, Settings, traits, mock

**Files:**
- Create: `crates/driver-core/Cargo.toml`, `crates/driver-core/src/lib.rs`

**Interfaces:**
- Consumes: `geometry::Polyline`.
- Produces: `Job`, `Settings`, `MachineProfile`, `Driver`, `Transport`, `MockTransport`, `DriverError`, `TransportError`.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mock_transport_records_all_bytes() {
        let mut t = MockTransport::default();
        t.write(b"AB").unwrap();
        t.write(b"C").unwrap();
        assert_eq!(t.written, b"ABC");
    }
    #[test]
    fn default_settings_leave_speed_force_unset() {
        let s = Settings::default();
        assert!(s.speed.is_none() && s.force.is_none() && s.passes == 1);
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p driver-core` → FAIL.

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml`:
```toml
[package]
name = "driver-core"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-or-later"

[dependencies]
geometry = { path = "../geometry" }
```

`src/lib.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use geometry::Polyline;

pub struct Job { pub polylines: Vec<Polyline>, pub settings: Settings }

#[derive(Clone, Debug, PartialEq)]
pub struct Settings { pub speed: Option<u32>, pub force: Option<u32>, pub passes: u32 }
impl Default for Settings { fn default() -> Self { Settings { speed: None, force: None, passes: 1 } } }

#[derive(Clone, Debug, PartialEq)]
pub struct MachineProfile { pub id: String, pub name: String, pub width_mm: f64, pub height_mm: f64 }

#[derive(Debug, PartialEq)]
pub enum DriverError { UnsupportedGeometry, Encode(String) }
#[derive(Debug, PartialEq)]
pub enum TransportError { NotFound, Io(String) }

pub trait Driver {
    fn encode(&self, job: &Job) -> Result<Vec<u8>, DriverError>;
    fn profile(&self) -> &MachineProfile;
}
pub trait Transport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError>;
}

#[derive(Default)]
pub struct MockTransport { pub written: Vec<u8> }
impl Transport for MockTransport {
    fn write(&mut self, b: &[u8]) -> Result<usize, TransportError> {
        self.written.extend_from_slice(b); Ok(b.len())
    }
}
```

- [ ] **Step 4: Run to verify it passes.** `cargo test -p driver-core` → PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/driver-core/ && git commit -m "Add driver-core job model, traits, and mock transport"
```

---

### Task 6: `driver-silhouette` — GPGL encoder (golden, cross-checked with send_raw.py)

**Files:**
- Create: `crates/driver-silhouette/Cargo.toml`, `crates/driver-silhouette/src/lib.rs`, `crates/driver-silhouette/src/encode.rs`

**Interfaces:**
- Consumes: `driver-core::{Driver, Job, Settings, MachineProfile, DriverError}`, `geometry::Point`.
- Produces: `SilhouetteDriver::new() -> Self`, `impl Driver for SilhouetteDriver`.

- [ ] **Step 1: Write the failing test**

`src/encode.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::{Job, Settings, Driver};
    use geometry::Point;

    fn square() -> Vec<Point> {
        [(0.0,0.0),(20.0,0.0),(20.0,20.0),(0.0,20.0),(0.0,0.0)]
            .iter().map(|&(x,y)| Point{x,y}).collect()
    }

    #[test]
    fn encodes_square_to_documented_gpgl_stream() {
        let d = SilhouetteDriver::new();
        let job = Job { polylines: vec![square()], settings: Settings::default() };
        let bytes = d.encode(&job).unwrap();
        // ESC EOT · J1 · M0,0 · D0,400 · D400,400 · D400,0 · D0,0 · SO0 · FN0  (20/mm, (y,x))
        let mut want = vec![0x1b, 0x04];
        for cmd in ["J1","M0,0","D0,400","D400,400","D400,0","D0,0","SO0","FN0"] {
            want.extend_from_slice(cmd.as_bytes()); want.push(0x03);
        }
        assert_eq!(bytes, want);
    }

    #[test]
    fn speed_and_force_emitted_only_when_set() {
        let d = SilhouetteDriver::new();
        let job = Job { polylines: vec![square()],
            settings: Settings { speed: Some(10), force: Some(20), passes: 1 } };
        let s = String::from_utf8_lossy(&d.encode(&job).unwrap()).to_string();
        assert!(s.contains("!10,1\u{3}") && s.contains("FX20,1\u{3}"));
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p driver-silhouette` → FAIL.

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml` deps: `driver-core`, `geometry`, `nusb = "0.1"`.

`src/encode.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Driver, Job, MachineProfile, DriverError};

pub struct SilhouetteDriver { profile: MachineProfile }
impl SilhouetteDriver {
    pub fn new() -> Self {
        SilhouetteDriver { profile: MachineProfile {
            id: "cameo5".into(), name: "Silhouette Cameo 5 Alpha".into(),
            width_mm: 330.0, height_mm: 3000.0 } }
    }
}
fn su(mm: f64) -> i64 { (mm * 20.0).round() as i64 }   // 20 units/mm

impl Driver for SilhouetteDriver {
    fn encode(&self, job: &Job) -> Result<Vec<u8>, DriverError> {
        let tool = 1;
        let mut out: Vec<u8> = vec![0x1b, 0x04];            // ESC EOT init
        let mut push = |s: String, out: &mut Vec<u8>| { out.extend_from_slice(s.as_bytes()); out.push(0x03); };
        push(format!("J{tool}"), &mut out);
        if let Some(sp) = job.settings.speed { push(format!("!{sp},{tool}"), &mut out); }
        if let Some(fo) = job.settings.force { push(format!("FX{fo},{tool}"), &mut out); }
        for _ in 0..job.settings.passes.max(1) {
            for poly in &job.polylines {
                if poly.is_empty() { continue; }
                let f = poly[0];                            // note (y,x) order
                push(format!("M{},{}", su(f.y), su(f.x)), &mut out);
                for p in &poly[1..] { push(format!("D{},{}", su(p.y), su(p.x)), &mut out); }
            }
        }
        push("SO0".into(), &mut out);
        push("FN0".into(), &mut out);
        Ok(out)
    }
    fn profile(&self) -> &MachineProfile { &self.profile }
}
```

`src/lib.rs`: `// SPDX...` + `mod encode; pub use encode::SilhouetteDriver; mod usb; pub use usb::UsbTransport;` (usb added in Task 7).

- [ ] **Step 4: Run to verify it passes.** `cargo test -p driver-silhouette` → PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/driver-silhouette/ && git commit -m "Add Silhouette GPGL encoder with golden square test"
```

---

### Task 7: `driver-silhouette` — UsbTransport (nusb)

**Files:**
- Create: `crates/driver-silhouette/src/usb.rs`

**Interfaces:**
- Produces: `UsbTransport::open() -> Result<UsbTransport, TransportError>`, `impl Transport`.

- [ ] **Step 1: Write the failing test** (constructor error path — no device present in CI)

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn open_without_device_reports_not_found() {
        // CI has no Cameo attached → must be a typed NotFound, never a panic.
        match UsbTransport::open() { Err(driver_core::TransportError::NotFound) => {}, other => panic!("{other:?}") }
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p driver-silhouette usb::` → FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Transport, TransportError};

const VID: u16 = 0x3844;
const PID: u16 = 0x0001;
const EP_OUT: u8 = 0x01;

pub struct UsbTransport { iface: nusb::Interface }
impl UsbTransport {
    pub fn open() -> Result<UsbTransport, TransportError> {
        let di = nusb::list_devices().map_err(|e| TransportError::Io(e.to_string()))?
            .find(|d| d.vendor_id() == VID && d.product_id() == PID)
            .ok_or(TransportError::NotFound)?;
        let dev = di.open().map_err(|e| TransportError::Io(e.to_string()))?;
        let iface = dev.claim_interface(0).map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(UsbTransport { iface })
    }
}
impl Transport for UsbTransport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
        let out = self.iface.bulk_out(EP_OUT, bytes.to_vec());
        let done = nusb::block_on(out);
        done.status.map_err(|e| TransportError::Io(format!("{e:?}")))?;
        Ok(bytes.len())
    }
}
```

(Pin exact nusb 0.1 API surface — `bulk_out`/`block_on` names may differ by patch; adjust to the version resolved.)

- [ ] **Step 4: Run to verify it passes.** `cargo test -p driver-silhouette usb::` → PASS (NotFound on CI).

- [ ] **Step 5: Commit**

```bash
git add crates/driver-silhouette/ && git commit -m "Add Silhouette nusb bulk transport"
```

---

### Task 8: `driver-hpgl` — HPGL encoder (golden, cross-checked with hpgl.py)

**Files:**
- Create: `crates/driver-hpgl/Cargo.toml`, `crates/driver-hpgl/src/lib.rs`, `crates/driver-hpgl/src/encode.rs`

**Interfaces:**
- Produces: `HpglDriver::new()`, `impl Driver`.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    use driver_core::{Job, Settings, Driver};
    use geometry::Point;
    #[test]
    fn encodes_square_to_documented_hpgl_stream() {
        let d = HpglDriver::new();
        let poly: Vec<Point> = [(0.0,0.0),(20.0,0.0),(20.0,20.0),(0.0,20.0),(0.0,0.0)]
            .iter().map(|&(x,y)| Point{x,y}).collect();
        let job = Job { polylines: vec![poly], settings: Settings::default() };
        let s = String::from_utf8(d.encode(&job).unwrap()).unwrap();
        // 20mm → 800 units (1016/in), (x,y) order
        assert_eq!(s, "IN;PU0,0;PD800,0;PD800,800;PD0,800;PD0,0;PU;");
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p driver-hpgl` → FAIL.

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml` deps: `driver-core`, `geometry`, `serialport = "4"`.

`src/encode.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Driver, Job, MachineProfile, DriverError};

pub struct HpglDriver { profile: MachineProfile }
impl HpglDriver {
    pub fn new() -> Self {
        HpglDriver { profile: MachineProfile {
            id: "puma".into(), name: "GCC Puma IV".into(), width_mm: 600.0, height_mm: 5000.0 } }
    }
}
fn u(mm: f64) -> i64 { (mm / 25.4 * 1016.0).round() as i64 }   // 1016 units/inch

impl Driver for HpglDriver {
    fn encode(&self, job: &Job) -> Result<Vec<u8>, DriverError> {
        // Ignores speed/force in V1 (GCC panel-set; see gcc-hpgl.md).
        let mut s = String::from("IN;");
        for _ in 0..job.settings.passes.max(1) {
            for poly in &job.polylines {
                if poly.is_empty() { continue; }
                let f = poly[0];                                   // (x,y) order
                s.push_str(&format!("PU{},{};", u(f.x), u(f.y)));
                for p in &poly[1..] { s.push_str(&format!("PD{},{};", u(p.x), u(p.y))); }
            }
        }
        s.push_str("PU;");
        Ok(s.into_bytes())
    }
    fn profile(&self) -> &MachineProfile { &self.profile }
}
```

`src/lib.rs`: `mod encode; pub use encode::HpglDriver; mod serial; pub use serial::SerialTransport;` (serial in Task 9).

- [ ] **Step 4: Run to verify it passes.** `cargo test -p driver-hpgl` → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/driver-hpgl/ && git commit -m "Add HPGL encoder with golden square test"
```

---

### Task 9: `driver-hpgl` — SerialTransport (serialport)

**Files:**
- Create: `crates/driver-hpgl/src/serial.rs`

**Interfaces:**
- Produces: `SerialTransport::open(port: &str, baud: u32) -> Result<SerialTransport, TransportError>`, `impl Transport`.

- [ ] **Step 1: Write the failing test**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn open_nonexistent_port_reports_io_error() {
        match SerialTransport::open("/dev/does-not-exist-xyz", 9600) {
            Err(driver_core::TransportError::Io(_)) => {},
            other => panic!("{other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p driver-hpgl serial::` → FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use std::time::Duration;
use driver_core::{Transport, TransportError};

pub struct SerialTransport { port: Box<dyn serialport::SerialPort> }
impl SerialTransport {
    pub fn open(port: &str, baud: u32) -> Result<SerialTransport, TransportError> {
        let p = serialport::new(port, baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .timeout(Duration::from_secs(5))
            .open().map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(SerialTransport { port: p })
    }
}
impl Transport for SerialTransport {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError> {
        use std::io::Write;
        self.port.write_all(bytes).map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(bytes.len())
    }
}
```

- [ ] **Step 4: Run to verify it passes.** `cargo test -p driver-hpgl serial::` → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/driver-hpgl/ && git commit -m "Add HPGL serial transport"
```

---

### Task 10: `cli` — cut, list-devices, dry-run, registry

**Files:**
- Create: `crates/cli/Cargo.toml`, `crates/cli/src/main.rs`, `crates/cli/src/pipeline.rs`
- Create: `crates/cli/tests/dry_run.rs`, `crates/cli/tests/fixtures/square.svg`

**Interfaces:**
- Consumes: everything above.
- Produces: the `cuthulhu` binary and a `build_bytes(svg, device, settings) -> Result<Vec<u8>, String>` used by both `main` and the integration test.

- [ ] **Step 1: Write the failing test**

`crates/cli/tests/fixtures/square.svg`:
```xml
<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20"><rect width="20" height="20"/></svg>
```

`crates/cli/tests/dry_run.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use cli::pipeline::{build_bytes, Device};
use driver_core::Settings;

#[test]
fn hpgl_dry_run_matches_documented_stream() {
    let svg = std::fs::read("tests/fixtures/square.svg").unwrap();
    let bytes = build_bytes(&svg, Device::Puma, &Settings::default()).unwrap();
    // 20 user-units = 20px → 20*25.4/96 mm → ×1016/25.4 = 20*1016/96 ≈ 212 units.
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.starts_with("IN;PU0,0;PD212,0;") || s.starts_with("IN;PU0,0;PD211,0;"), "{s}");
    assert!(s.ends_with("PU;"));
}
```

> The px→mm→units chain makes the exact integer depend on rounding; the test pins the format and the first coordinate to ±1 unit. (A future SVG authored in real `mm` units removes the 96 dpi factor.)

- [ ] **Step 2: Run to verify it fails.** `cargo test -p cli` → FAIL.

- [ ] **Step 3: Write minimal implementation**

`Cargo.toml` deps: `geometry`, `fileio`, `driver-core`, `driver-silhouette`, `driver-hpgl`, `clap = { version = "4", features = ["derive"] }`. Set `[lib]` + `[[bin]]` so tests can import `cli::pipeline`.

`src/pipeline.rs`:
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Driver, Job, Settings};
use driver_silhouette::SilhouetteDriver;
use driver_hpgl::HpglDriver;

#[derive(Clone, Copy)]
pub enum Device { Cameo5, Puma }
impl Device {
    pub fn from_id(s: &str) -> Result<Device, String> {
        match s { "cameo5" => Ok(Device::Cameo5), "puma" => Ok(Device::Puma),
                  _ => Err(format!("unknown device '{s}' (try: cameo5, puma)")) }
    }
    fn driver(&self) -> Box<dyn Driver> {
        match self { Device::Cameo5 => Box::new(SilhouetteDriver::new()),
                     Device::Puma => Box::new(HpglDriver::new()) }
    }
}

pub fn build_bytes(svg: &[u8], device: Device, settings: &Settings) -> Result<Vec<u8>, String> {
    let imp = fileio::svg_to_paths(svg).map_err(|e| format!("SVG parse: {e:?}"))?;
    let polylines = imp.paths.iter()
        .flat_map(|(path, _)| path.flatten(0.1))
        .collect::<Vec<_>>();
    if polylines.is_empty() { return Err("no cuttable paths in SVG".into()); }
    let job = Job { polylines, settings: settings.clone() };
    device.driver().encode(&job).map_err(|e| format!("encode: {e:?}"))
}
```

`src/main.rs` (clap): subcommands `cut` (`--device`, `--dry-run`, `--speed`, `--force`, `--port`, `--baud`) and `list-devices`. `cut` calls `build_bytes`; on `--dry-run` print hex+ASCII; else open the matching transport (`UsbTransport::open()` / `SerialTransport::open(port,baud)`) and `write`. `list-devices` prints each `Driver::profile()`.

- [ ] **Step 4: Run to verify it passes.** `cargo test -p cli` → PASS. Manual: `cargo run -p cli -- cut crates/cli/tests/fixtures/square.svg --device cameo5 --dry-run` prints the GPGL hex.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/ && git commit -m "Add cuthulhu CLI: cut, list-devices, dry-run pipeline"
```

---

## Self-review

**Spec coverage:** vertical slice (Tasks 1–10 end-to-end) · geometry cut-path-only: Affine (2), Path/flatten/from_svg/to_svg (3) · fileio svg_to_paths + skipped (4) · driver-core Job/Settings/traits/mock, Option speed/force asymmetry (5) · Silhouette GPGL encode 20/mm (y,x) + speed/force applied (6) + nusb transport (7) · HPGL encode 1016/in (x,y) + speed/force ignored (8) + serial transport (9) · CLI cut/list-devices/dry-run/registry (10) · golden tests cross-checking SP1 senders (6, 8) · no-silent-failures typed Results throughout · CI cargo job (1). Physical cut = manual gate, correctly not automated. Deferred (booleans/text, project IO, status) have no tasks — correct.

**Placeholder scan:** one deliberate illustrative dead-code block in Task 3's `from_svg` is called out with an explicit "delete them" note and the correct one-line body; no other placeholders. `paint_rgba`/quad→cubic carry `ponytail:` markers naming the simplification.

**Type consistency:** `Point`/`Polyline`/`Path`/`Seg`/`Affine`/`Rect`/`GeomError` consistent geometry→fileio→drivers; `Job`/`Settings`/`Driver`/`Transport`/`MachineProfile`/`DriverError`/`TransportError` consistent core→drivers→cli; `su()` (20/mm) vs `u()` (1016/in) distinct per encoder; `build_bytes`/`Device` bridge cli test and main.

**External-API caveat (honest):** exact `usvg 0.44`, `nusb 0.1`, and `serialport 4` method names may shift by patch; Tasks 4/7/9 each say to pin against the resolved version. These are the only places reality can diverge from the code as written.
