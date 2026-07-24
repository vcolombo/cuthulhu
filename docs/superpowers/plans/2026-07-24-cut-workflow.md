# SP4 Cut Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The GUI (and CLI) drives the SP2 drivers: pick a device, pick a material preset, preview passes, and cut multi-color documents as ordered passes with operator-confirmed color swaps — per `docs/superpowers/specs/2026-07-24-cut-workflow-design.md`.

**Architecture:** New `cutplan` crate (planning, preflight, presets) + a concrete-driver-free `DeviceManager` in `driver-core` (worker thread, bounded command channel, event stream with job ids, session-level encoding). Desktop and CLI inject a `DeviceBackendFactory`. Tauri layer is a thin IPC + event bridge with its own managed handle (never the document mutex).

**Tech Stack:** Rust (std mpsc/threads, no async runtime), existing rusb/serialport transports, Tauri 2 events, React + TS + Vitest, Playwright.

## Global Constraints

- Every new source file starts with `// SPDX-License-Identifier: GPL-3.0-or-later`.
- No AI attribution in commits or code.
- Units: mm (f64) everywhere outside drivers; device-unit conversion only inside drivers.
- No silent failures: typed errors end-to-end; `PassComplete` means the machine finished moving (device-confirmed on Cameo, operator-confirmed on Puma), never "bytes accepted"; progress is **bytes submitted**, labeled "sending"; state never claims `Idle`/`Cancelled` while the machine may still be moving.
- Session framing: exactly one prologue (`session_begin`) and one epilogue (`session_end`) per multi-pass job; between-pass swaps are tool/blade only.
- Cancellation is cooperative/best-effort; bounded write timeouts; `write_all` semantics (`Ok(0)` → `WriteZero`).
- Canonical machine ids: `cameo5`, `puma`. Legacy `cameo5_alpha`/`puma_iv` map on project load.
- `repeat_count` (renamed from `Settings.passes`) validated `1..=10`; zero rejected, never coerced.
- Every `DeviceEvent` carries `job_id`; UI drops stale-job events.
- New IPC uses `IpcError { code, message }`; existing 15 editor commands keep their `Result<T, String>` shape.
- The Playwright smoke's existing assertions must not be weakened.
- Workbench tokens unchanged; dialog errors in `--cut` red.

## Existing interfaces consumed (verified)

```rust
// driver-core (pre-plan state)
pub struct Job { pub polylines: Vec<Polyline>, pub settings: Settings }
pub struct Settings { pub speed: Option<u32>, pub force: Option<u32>, pub passes: u32 }
pub trait Driver { fn encode(&self, job:&Job)->Result<Vec<u8>,DriverError>; fn profile(&self)->&MachineProfile; }
pub trait Transport { fn write(&mut self, bytes:&[u8])->Result<usize,TransportError>; }
pub struct MockTransport { pub written: Vec<u8> }
// driver-silhouette encode: [0x1b,0x04] prologue; "J1","!{speed},1","FX{force},1"; "M{y},{x}"/"D{y},{x}" (0x03-terminated, su=mm*20, (y,x) order); epilogue "SO0" (+ trailing block — read the file); usb.rs opens first VID/PID match.
// driver-hpgl encode: "IN;" prologue; "PU{x},{y};PD..."(u=mm/25.4*1016); ignores speed/force (panel-set); serial.rs.
// document: Document (snapshot_json, root, ids, artboard, machine), NodeId, ShapeKind{Rect,Ellipse,Text,Path}, Style{stroke,fill:Option<u32> 0xRRGGBBAA}, world composition a.then(b)=a-first, commands::local_shape_path (PRIVATE, no Text arm), machine::builtin_profiles (ids cameo5_alpha/puma_iv — Task 2 fixes).
// fileio: svg_to_paths (paint_rgba hardcoded black — Task 1 fixes), import_svg (stroke defaults to black — Task 1 fixes), save_project/load_project, doc_to_svg.
// geometry: Path::flatten(tol)->Vec<Polyline>, Polyline=Vec<Point>, text_to_path(family,size,text).
// cli: pipeline.rs Device::from_id/driver() registry; main.rs clap.
// apps/desktop: ipc.rs #[tauri::command] over Mutex<AppState>; ui invoke wrappers snake_case; e2e mock __TAURI_INTERNALS__.
```

---

### Task 1: fileio — real SVG stroke colors

**Files:**
- Modify: `crates/fileio/src/lib.rs` (`paint_rgba`), `crates/fileio/src/import.rs`

**Interfaces:**
- Produces: `svg_to_paths` StyleHints carry real resolved RGBA stroke/fill; `import_svg` preserves `stroke: None` (no manufactured black). Later tasks rely on multi-color documents being importable.

- [ ] **Step 1: Write the failing test** (in `lib.rs` tests)

```rust
#[test]
fn import_preserves_stroke_colors_and_none() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="10">
        <rect width="10" height="10" stroke="#ff0000" fill="none"/>
        <rect x="10" width="10" height="10" stroke="#0000ff" fill="none"/>
        <rect x="20" width="10" height="10" fill="#00ff00"/></svg>"##;
    let imp = svg_to_paths(svg).unwrap();
    let strokes: Vec<Option<u32>> = imp.paths.iter().map(|(_, h)| h.stroke).collect();
    assert!(strokes.contains(&Some(0xFF0000FF)), "red stroke preserved: {strokes:?}");
    assert!(strokes.contains(&Some(0x0000FFFF)), "blue stroke preserved");
    assert!(strokes.contains(&None), "no-stroke shape stays None");
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p fileio import_preserves` → FAIL (all strokes 0x000000FF).

- [ ] **Step 3: Implement.** Replace `paint_rgba`:

```rust
fn paint_rgba(paint: &usvg::Paint, opacity: usvg::Opacity) -> u32 {
    match paint {
        usvg::Paint::Color(c) => {
            let a = (opacity.get() * 255.0).round() as u32;
            ((c.red as u32) << 24) | ((c.green as u32) << 16) | ((c.blue as u32) << 8) | a
        }
        // Gradients/patterns can't drive a blade; fall back to opaque black so the shape still cuts visibly.
        _ => 0x000000FF,
    }
}
```

Update its call sites to pass `p.stroke().map(|s| paint_rgba(s.paint(), s.opacity()))` / same for fill. In `import.rs`, change `Style { stroke: hint.stroke.or(Some(0x000000FF)), fill: hint.fill }` to `Style { stroke: hint.stroke, fill: hint.fill }` — `None` stays `None`. Check existing fileio/document tests that assumed the black default and update their expectations to match real colors (the SP3 smoke mock is unaffected — it doesn't go through usvg).

- [ ] **Step 4: Run to verify pass.** `cargo test -p fileio && cargo test --workspace` → all pass, zero warnings.

- [ ] **Step 5: Commit.** `git add crates/fileio/ && git commit -m "Preserve real SVG stroke colors on import"`

---

### Task 2: document — canonical machine ids + legacy migration

**Files:**
- Modify: `crates/document/src/machine.rs`, `crates/fileio/src/project.rs`

**Interfaces:**
- Produces: `builtin_profiles()` ids become `"cameo5"` / `"puma"` (matching driver profiles). `load_project` maps legacy ids `cameo5_alpha→cameo5`, `puma_iv→puma` in the loaded `Document.machine`.

- [ ] **Step 1: Write the failing tests**

```rust
// machine.rs
#[test]
fn builtin_ids_are_canonical() {
    let ids: Vec<String> = builtin_profiles().into_iter().map(|p| p.id).collect();
    assert_eq!(ids, vec!["cameo5", "puma"]);
}
// project.rs
#[test]
fn legacy_machine_ids_migrate_on_load() {
    let mut doc = document::Document::new();
    let legacy = document::MachineProfile { id: "puma_iv".into(), name: "GCC Puma IV".into(),
        width_mm: 600.0, height_mm: 5000.0 };
    doc.machine = Some(legacy);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.cut");
    save_project(&path, &doc).unwrap();
    let back = load_project(&path).unwrap();
    assert_eq!(back.machine.unwrap().id, "puma");
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p document machine:: -p fileio legacy_machine` → FAIL.

- [ ] **Step 3: Implement.** In `machine.rs` change ids to `"cameo5"` / `"puma"` (names unchanged). In `project.rs::load_project`, after deserializing:

```rust
    if let Some(m) = doc.machine.as_mut() {
        m.id = match m.id.as_str() {
            "cameo5_alpha" => "cameo5".into(),
            "puma_iv" => "puma".into(),
            _ => std::mem::take(&mut m.id),
        };
    }
```

Fix any tests referencing old ids (`set_machine_resizes_artboard` uses `puma_iv` — update to `puma`; grep both old ids workspace-wide incl. `apps/desktop`).

- [ ] **Step 4: Run to verify pass.** `cargo test --workspace` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/ apps/ && git commit -m "Adopt canonical machine ids with legacy project migration"`

---

### Task 3: driver-core — session encoding contract + MachineCaps + repeat_count

**Files:**
- Modify: `crates/driver-core/src/lib.rs`, `crates/driver-silhouette/src/encode.rs`, `crates/driver-hpgl/src/encode.rs`, `crates/cli/src/pipeline.rs`
- Golden test files: keep/extend the drivers' existing golden-byte tests

**Interfaces:**
- Produces (later tasks depend on these exact signatures):

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MachineCaps { pub supports_speed: bool, pub supports_force: bool, pub needs_operator_pass_confirm: bool }

pub struct Settings { pub speed: Option<u32>, pub force: Option<u32>, pub repeat_count: u32 }  // renamed field

pub trait Driver {
    fn profile(&self) -> &MachineProfile;
    fn caps(&self) -> MachineCaps;
    fn session_begin(&self) -> Vec<u8>;
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError>;
    fn pass_park(&self) -> Vec<u8>;
    fn session_end(&self) -> Vec<u8>;
    fn abort_bytes(&self) -> Option<Vec<u8>>;
}
```

- [ ] **Step 1: Write the failing tests** (silhouette `encode.rs` tests; mirror for hpgl)

```rust
#[test]
fn session_framing_has_one_prologue_and_one_epilogue_across_two_passes() {
    let d = SilhouetteDriver::new();
    let job = |force| Job { polylines: vec![vec![Point{x:0.0,y:0.0}, Point{x:10.0,y:0.0}]],
                            settings: Settings { speed: Some(5), force: Some(force), repeat_count: 1 } };
    let mut bytes = d.session_begin();
    bytes.extend(d.encode_pass(&job(10)).unwrap());
    bytes.extend(d.pass_park());
    bytes.extend(d.encode_pass(&job(20)).unwrap());
    bytes.extend(d.session_end());
    let count = |needle: &[u8]| bytes.windows(needle.len()).filter(|w| *w == needle).count();
    assert_eq!(count(&[0x1b, 0x04]), 1, "exactly one ESC EOT prologue");
    assert_eq!(count(b"SO0"), 1, "exactly one feed-out epilogue");
    assert_eq!(count(b"FX10,1"), 1);
    assert_eq!(count(b"FX20,1"), 1, "per-pass settings present");
}

#[test]
fn single_pass_session_is_byte_identical_to_sp2_encoding() {
    let d = SilhouetteDriver::new();
    let job = Job { polylines: vec![vec![Point{x:1.0,y:2.0}, Point{x:3.0,y:4.0}]],
                    settings: Settings { speed: Some(8), force: Some(12), repeat_count: 2 } };
    let mut session = d.session_begin();
    session.extend(d.encode_pass(&job).unwrap());
    session.extend(d.session_end());
    // must equal the pre-plan golden bytes for this job (copy the expected Vec from the
    // existing SP2 golden test for the same input before changing the encoder)
    assert_eq!(session, sp2_golden_for_job());
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p driver-silhouette` → FAIL (methods missing / field renamed).

- [ ] **Step 3: Implement.**
  - `driver-core`: rename `passes` → `repeat_count` (Default = 1); add `MachineCaps`; replace `Driver` with the trait above (delete the old `encode`).
  - `driver-silhouette`: `session_begin` = `vec![0x1b, 0x04]`; `encode_pass` = the current body minus prologue and minus `SO0…` trailer (settings + `repeat_count` loops of `M`/`D`); `session_end` = the current trailer exactly (`SO0` + whatever follows it in the existing encoder — preserve byte-for-byte); `pass_park` = `Vec::new()` with comment `// ponytail: no documented safe-park command yet; head stays put between passes — hardware checklist validates`; `abort_bytes` = `None` (undocumented); `caps` = `{ supports_speed: true, supports_force: true, needs_operator_pass_confirm: false }`.
  - `driver-hpgl`: `session_begin` = `b"IN;".to_vec()`; `encode_pass` = current body minus `IN;` and minus its trailer; `session_end` = the current trailer exactly (read the existing encoder's end — preserve byte-for-byte); `pass_park` = `b"PU;".to_vec()`; `abort_bytes` = `Some(b"PU;".to_vec())` (queued best-effort pen-up); `caps` = `{ supports_speed: false, supports_force: false, needs_operator_pass_confirm: true }`.
  - `cli/pipeline.rs`: `build_bytes` becomes `session_begin + encode_pass + session_end`.
  - Update every `passes:` field reference workspace-wide (`grep -rn "passes" crates/ apps/desktop/src`).

- [ ] **Step 4: Run to verify pass.** `cargo test --workspace` → all pass (byte-identity proves no CLI regression), zero warnings.
- [ ] **Step 5: Commit.** `git add crates/ && git commit -m "Split driver encoding into session/pass/park phases with machine caps"`

---

### Task 4: driver-core — Transport read + write_all contract

**Files:**
- Modify: `crates/driver-core/src/lib.rs` (Transport, MockTransport, TransportError), `crates/driver-silhouette/src/usb.rs`, `crates/driver-hpgl/src/serial.rs`

**Interfaces:**
- Produces:

```rust
pub enum TransportError { NotFound, Timeout, WriteZero, Io(String) }
pub trait Transport: Send {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, TransportError>;
    fn read(&mut self, buf: &mut [u8], timeout: std::time::Duration) -> Result<usize, TransportError>;
}
pub fn write_all(t: &mut dyn Transport, mut bytes: &[u8]) -> Result<(), TransportError>;  // loops; Ok(0) → WriteZero
// MockTransport gains scripting:
pub struct MockTransport {
    pub written: Vec<u8>,
    pub reads: std::collections::VecDeque<Result<Vec<u8>, TransportError>>, // scripted read results
    pub write_results: std::collections::VecDeque<Result<usize, TransportError>>, // scripted overrides; default = full write
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn write_all_loops_partial_writes_and_flags_zero() {
    let mut t = MockTransport::default();
    t.write_results.push_back(Ok(2)); // partial: only 2 of 5 accepted
    write_all(&mut t, b"HELLO").unwrap();
    assert_eq!(t.written, b"HELLO");

    let mut z = MockTransport::default();
    z.write_results.push_back(Ok(0));
    assert_eq!(write_all(&mut z, b"X"), Err(TransportError::WriteZero));
}

#[test]
fn mock_read_replays_script_then_times_out() {
    let mut t = MockTransport::default();
    t.reads.push_back(Ok(b"ready".to_vec()));
    let mut buf = [0u8; 8];
    let n = t.read(&mut buf, std::time::Duration::from_millis(10)).unwrap();
    assert_eq!(&buf[..n], b"ready");
    assert_eq!(t.read(&mut buf, std::time::Duration::from_millis(10)), Err(TransportError::Timeout));
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p driver-core` → FAIL.
- [ ] **Step 3: Implement** per the interface block: `MockTransport::write` pops `write_results` (empty deque = accept all `bytes.len()`), appends the accepted prefix to `written`; `read` pops `reads` (empty = `Timeout`). `write_all` loops slices, mapping `Ok(0)` to `WriteZero`. `usb.rs`: add `read` on bulk-in EP `0x82` with the passed timeout (rusb `read_bulk`); map rusb timeout errors to `Timeout`. `serial.rs`: `read` with `set_timeout` + `std::io::Read`, timeout → `Timeout`. `Transport: Send` bound may require adding `Send` to concrete types — both wrap owned handles, so derive/impl straightforwardly.
- [ ] **Step 4: Run to verify pass.** `cargo test --workspace` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/ && git commit -m "Add transport reads and write_all with typed timeout errors"`

---

### Task 5: driver-core — DeviceInfo + DeviceBackendFactory; CLI registry rework

**Files:**
- Modify: `crates/driver-core/src/lib.rs`, `crates/cli/src/pipeline.rs`, `crates/driver-silhouette/src/usb.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, PartialEq)]
pub enum TransportKind { Usb { locator: String }, Serial { path: String, baud: u32 } }  // locator "bus:address"
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo { pub instance_id: String, pub machine_id: String, pub transport: TransportKind, pub candidate: bool }

pub trait DeviceBackendFactory: Send + Sync {
    fn list_devices(&self) -> Vec<DeviceInfo>;
    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>>;
    fn open_transport(&self, info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError>;
}
```

- [ ] **Step 1: Write the failing test** (driver-core, with a test factory)

```rust
struct FakeFactory;
impl DeviceBackendFactory for FakeFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> {
        vec![DeviceInfo { instance_id: "usb:1:4".into(), machine_id: "cameo5".into(),
                          transport: TransportKind::Usb { locator: "1:4".into() }, candidate: false },
             DeviceInfo { instance_id: "serial:/dev/ttyUSB0".into(), machine_id: "puma".into(),
                          transport: TransportKind::Serial { path: "/dev/ttyUSB0".into(), baud: 9600 }, candidate: true }]
    }
    fn driver_for(&self, _: &str) -> Option<Box<dyn Driver + Send>> { None }
    fn open_transport(&self, _: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> { Err(TransportError::NotFound) }
}
#[test]
fn serial_devices_are_candidates_requiring_user_selection() {
    let f = FakeFactory;
    let serial: Vec<_> = f.list_devices().into_iter().filter(|d| matches!(d.transport, TransportKind::Serial{..})).collect();
    assert!(serial.iter().all(|d| d.candidate), "serial ports can't be assumed to be Pumas");
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p driver-core candidates` → FAIL (types missing).
- [ ] **Step 3: Implement** the types/trait in driver-core (test compiles = the contract exists). Rework `cli/pipeline.rs`: its `Device` registry becomes a `CliBackendFactory` implementing the trait — USB enumeration via rusb VID/PID scan producing `locator = "{bus}:{address}"`, serial enumeration via `serialport::available_ports` (all `candidate: true`, default baud 9600 per protocol notes). `usb.rs` gains `open_at(locator: &str)` that matches bus/address instead of first-match (keep the old `open()` delegating to the first enumerated locator for CLI back-compat). CLI `cut` command keeps working via the factory.
- [ ] **Step 4: Run to verify pass.** `cargo test --workspace` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/ && git commit -m "Add device enumeration contract with injectable backend factory"`

---

### Task 6: driver-core — DeviceManager: lifecycle (connect/disconnect/snapshot/shutdown)

**Files:**
- Create: `crates/driver-core/src/manager.rs`
- Modify: `crates/driver-core/src/lib.rs` (`pub mod manager;`)

**Interfaces:**
- Produces (Tasks 7–8, 12, 15 build on these exact types):

```rust
pub enum DeviceState {
    Disconnected, Connecting, Idle,
    Transmitting { job_id: u64, pass_index: usize, submitted_bytes: usize, total_bytes: usize },
    AwaitingCompletion { job_id: u64, pass_index: usize },
    WaitingForColorSwap { job_id: u64, next_pass_index: usize },
    CancelRequested { job_id: u64 }, Stopping { job_id: u64 },
    Cancelled { job_id: u64, pass_index: usize, submitted_bytes: usize, completion_known: bool },
    Disconnecting, Error(DeviceError),
}
pub enum DeviceError { Disconnected, Busy, Timeout, WriteZero, Io(String) }
pub struct DeviceEvent { pub job_id: u64, pub kind: DeviceEventKind }
pub enum DeviceEventKind { StateChanged(DeviceState), Progress { pass_index: usize, submitted_bytes: usize, total_bytes: usize },
                           PassComplete(usize), JobComplete, Failed(DeviceError) }
pub struct CutPass { pub job: Job }                       // one per color, in configured order
pub struct DeviceManager { /* command Sender + JoinHandle */ }
impl DeviceManager {
    pub fn spawn(factory: std::sync::Arc<dyn DeviceBackendFactory>) -> (DeviceManager, std::sync::mpsc::Receiver<DeviceEvent>);
    pub fn connect(&self, info: DeviceInfo) -> Result<(), DeviceError>;      // sync reply; Connecting→Idle or Error
    pub fn disconnect(&self) -> Result<(), DeviceError>;
    pub fn snapshot(&self) -> DeviceState;                                    // for dialog open
    pub fn cut(&self, passes: Vec<CutPass>) -> Result<u64, DeviceError>;     // returns job_id (Task 7)
    pub fn cancel(&self);                                                     // (Task 8)
    pub fn resume(&self) -> Result<(), DeviceError>;                          // (Task 7)
    pub fn confirm_pass_done(&self) -> Result<(), DeviceError>;               // (Task 7)
    pub fn shutdown(self);                                                    // joins worker, bounded wait
}
```

Lifecycle-only in this task: `cut/cancel/resume/confirm_pass_done` exist but return/act `Err(DeviceError::Busy)`/no-op stubs documented as completed by Tasks 7–8. Worker owns `Option<Box<dyn Transport>>` + `Option<Box<dyn Driver + Send>>`; commands arrive on a bounded (`sync_channel(16)`) channel with per-command reply senders.

- [ ] **Step 1: Write the failing tests** (in `manager.rs`; use `FakeFactory` from Task 5 extended so `open_transport` returns a `MockTransport` and `driver_for` returns the silhouette driver)

```rust
#[test]
fn connect_transitions_disconnected_to_idle_and_events_fire() {
    let (mgr, events) = DeviceManager::spawn(std::sync::Arc::new(test_factory()));
    assert!(matches!(mgr.snapshot(), DeviceState::Disconnected));
    mgr.connect(cameo_info()).unwrap();
    assert!(matches!(mgr.snapshot(), DeviceState::Idle));
    let kinds: Vec<_> = events.try_iter().collect();
    assert!(kinds.iter().any(|e| matches!(e.kind, DeviceEventKind::StateChanged(DeviceState::Connecting))));
    assert!(kinds.iter().any(|e| matches!(e.kind, DeviceEventKind::StateChanged(DeviceState::Idle))));
    mgr.shutdown();
}
#[test]
fn connect_failure_yields_error_state_and_reconnect_recovers() {
    let (mgr, _events) = DeviceManager::spawn(std::sync::Arc::new(failing_open_factory()));
    assert!(mgr.connect(cameo_info()).is_err());
    assert!(matches!(mgr.snapshot(), DeviceState::Error(_)));
    // recovery path: a later successful connect clears Error
}
#[test]
fn double_connect_is_busy_and_shutdown_joins() {
    let (mgr, _e) = DeviceManager::spawn(std::sync::Arc::new(test_factory()));
    mgr.connect(cameo_info()).unwrap();
    assert_eq!(mgr.connect(cameo_info()).unwrap_err(), DeviceError::Busy);
    mgr.shutdown(); // must return (join), not hang
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p driver-core manager::` → FAIL.
- [ ] **Step 3: Implement** worker loop: `recv` command → mutate state → send `StateChanged` events → reply. `shutdown` sends a `Shutdown` command then `join`s with a 5s bound (log + detach on timeout). Dropped event receiver must not panic the worker (`let _ = events.send(..)`).
- [ ] **Step 4: Run to verify pass.** `cargo test -p driver-core` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/driver-core/ && git commit -m "Add device manager lifecycle with worker thread and event stream"`

---

### Task 7: driver-core — DeviceManager: cut flow (transmit → completion → swap → resume)

**Files:**
- Modify: `crates/driver-core/src/manager.rs`

**Interfaces:**
- Consumes: session contract (Task 3), `write_all` (Task 4). Completion policy: `caps().needs_operator_pass_confirm == false` → poll status: write ENQ `[0x05]`, `read` until response byte `b'0'` ("ready", per `docs/protocol/silhouette-cameo5.md` status replies), 250ms interval, 60s timeout → `Timeout`; `true` → enter `AwaitingCompletion` until `confirm_pass_done()`.
- Produces: working `cut`, `resume`, `confirm_pass_done` per Task 6 signatures; `job_id` monotonically increases per `cut`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn two_pass_job_frames_session_once_and_pauses_for_swap() {
    let (mgr, events) = DeviceManager::spawn(std::sync::Arc::new(test_factory_with_ready_reads(2)));
    mgr.connect(cameo_info()).unwrap();
    let job_id = mgr.cut(two_pass_job()).unwrap();
    wait_for_state(&mgr, |s| matches!(s, DeviceState::WaitingForColorSwap{..}));
    mgr.resume().unwrap();
    wait_for_state(&mgr, |s| matches!(s, DeviceState::Idle));
    let evs: Vec<_> = drain(events);
    assert!(evs.iter().all(|e| e.job_id == job_id));
    assert_eq!(evs.iter().filter(|e| matches!(e.kind, DeviceEventKind::PassComplete(_))).count(), 2);
    assert!(evs.iter().any(|e| matches!(e.kind, DeviceEventKind::JobComplete)));
    let written = mgr_written_bytes(); // helper: factory keeps Arc<Mutex<Vec<u8>>> mirror of MockTransport writes
    assert_eq!(count_subseq(&written, &[0x1b, 0x04]), 1, "one prologue for the whole job");
    assert_eq!(count_subseq(&written, b"SO0"), 1, "one epilogue for the whole job");
}
#[test]
fn resume_outside_swap_state_is_busy_and_cut_while_active_is_busy() { /* Idle: resume()→Busy; during Transmitting: cut()→Busy */ }
#[test]
fn operator_confirm_path_for_puma_caps() {
    // factory returns hpgl driver (needs_operator_pass_confirm: true): pass ends in AwaitingCompletion
    // until confirm_pass_done(); no status reads attempted (MockTransport reads stay empty ⇒ would Timeout if polled)
}
#[test]
fn stale_job_events_are_distinguishable() { /* run job 1 to completion, start job 2; assert event job_ids differ */ }
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p driver-core manager::` → FAIL.
- [ ] **Step 3: Implement.** `cut`: build byte plan (`session_begin`, per-pass `encode_pass` + `pass_park` between, `session_end` after last), total = sum; write in 4KB chunks via `write_all`, emitting `Progress` per chunk; after a pass's bytes: completion policy (ENQ poll or `AwaitingCompletion`); then `PassComplete(i)`; if more passes → `WaitingForColorSwap` until `resume()`; after final pass + `session_end` → `JobComplete`, `Idle`. Any transport error → `Failed` + `Error` state.
- [ ] **Step 4: Run to verify pass.** `cargo test -p driver-core` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/driver-core/ && git commit -m "Drive multi-pass cuts through session framing with completion policies"`

---

### Task 8: driver-core — DeviceManager: cancel + failure paths

**Files:**
- Modify: `crates/driver-core/src/manager.rs`

**Interfaces:**
- Produces: working `cancel()` per spec: `CancelRequested → Stopping → Cancelled { completion_known }`; `Cancelled.completion_known = true` only when device-ready confirmed (ENQ ready) — operator-confirm machines report `false`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn cancel_mid_transmit_stops_writes_sends_abort_and_confirms_stop() {
    // factory: large single-pass job (many chunks), scripted ready read for the post-abort ENQ
    // after cancel(): no further payload chunks written; hpgl abort "PU;" present for puma factory;
    // final state Cancelled { completion_known: true } for cameo (ready read), false for puma
}
#[test]
fn transport_write_error_mid_job_fails_loudly() {
    // write_results scripted: ok, ok, Err(Io) → Failed event with DeviceError::Io, state Error, no JobComplete
}
#[test]
fn write_zero_maps_to_typed_error() { /* Ok(0) script → Failed(WriteZero) */ }
#[test]
fn unplug_during_each_active_state_reports_disconnected() {
    // iterate: Transmitting, AwaitingCompletion, WaitingForColorSwap — script NotFound/Io errors at that point;
    // assert Error(Disconnected or Io) naming state, worker survives for reconnect
}
#[test]
fn shutdown_mid_job_cancels_and_joins() { /* spawn, cut, shutdown() returns within bound */ }
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p driver-core manager::` → FAIL.
- [ ] **Step 3: Implement.** `cancel()` sets `AtomicBool` (checked between chunks) + wakes blocked recv paths via the bounded write timeout (all transport writes already time-bound from Task 4 — pass `Duration::from_secs(5)` write deadline by chunking); on observing the flag: state `Stopping`, write `abort_bytes` if `Some`, run completion policy (ready-poll or skip), state `Cancelled{..}` then `Idle` on next snapshot read. Map `rusb`/`serialport` unplug errors to `Disconnected` where distinguishable, else `Io`.
- [ ] **Step 4: Run to verify pass.** `cargo test -p driver-core` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/driver-core/ && git commit -m "Add cooperative cancellation and typed failure paths to the device manager"`

---

### Task 9: document pub outline + `cutplan` crate — plan_passes / travel_moves

**Files:**
- Modify: `crates/document/src/commands.rs` (make outline conversion pub + Text arm)
- Create: `crates/cutplan/Cargo.toml`, `crates/cutplan/src/lib.rs`, `crates/cutplan/src/passes.rs`

**Interfaces:**
- Consumes: `document::{Document, NodeId, ShapeKind, NodeKind}`, `geometry::{Path, Polyline, Point, text_to_path}`.
- Produces:

```rust
// document (rename + publicize):
pub fn shape_outline(node: &Node) -> Result<Option<geometry::Path>, String>;
// None for containers; Text converts via text_to_path (family/size/text); Err carries font/parse detail.

// cutplan::passes:
pub struct PlannedShape { pub node_id: NodeId, pub polylines: Vec<Polyline> }
pub struct ColorPass { pub color: Option<u32>, pub shapes: Vec<PlannedShape> }
pub struct PlannedCut { pub passes: Vec<ColorPass>, pub skipped_no_stroke: usize, pub doc_revision: u64 }
#[derive(Debug, PartialEq)]
pub enum PlanError { BadShape(NodeId, String), MissingNode(NodeId), CycleDetected }
pub fn doc_revision(doc: &Document) -> u64;   // DefaultHasher over snapshot_json
pub fn plan_passes(doc: &Document) -> Result<PlannedCut, PlanError>;
pub fn travel_moves(configured: &[&ColorPass]) -> Vec<(Point, Point)>;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn plans_group_by_stroke_rgba_with_single_traversal_transforms() {
    // doc: group translate(10,0) containing red rect; root-level red rect at origin; blue ellipse; stroke-None rect
    let planned = plan_passes(&doc).unwrap();
    assert_eq!(planned.passes.len(), 2, "red + blue; None excluded");
    assert_eq!(planned.skipped_no_stroke, 1);
    let red = &planned.passes[0];       // first-seen order
    assert_eq!(red.shapes.len(), 2);
    // grouped child's polyline reflects the group translate (world transform applied)
    assert!(red.shapes.iter().any(|s| s.polylines[0][0].x >= 10.0));
}
#[test]
fn text_plans_as_glyph_outlines_or_typed_error() { /* Text node with any_available_family → non-empty polylines; bogus family → PlanError::BadShape */ }
#[test]
fn stale_revision_detectable() {
    let planned = plan_passes(&doc).unwrap();
    mutate(&mut doc);
    assert_ne!(planned.doc_revision, doc_revision(&doc));
}
#[test]
fn travel_moves_follow_configured_order() {
    // two passes reordered: segments connect end of each shape's last polyline to start of the next shape's first,
    // across the configured (reversed) pass order
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p cutplan` → FAIL (crate missing).
- [ ] **Step 3: Implement.** `document::shape_outline`: today's `local_shape_path` logic + `ShapeKind::Text` via `geometry::text_to_path(family, size_mm, text)` (errors stringified); keep a thin private alias where `boolean_op` used the old name. `plan_passes`: iterative preorder from `doc.root` with an explicit stack of `(NodeId, Affine)` carrying the accumulated transform; a `visited` HashSet detects cycles (`CycleDetected`); missing child id → `MissingNode`; shapes flatten `outline.transformed(&acc)` then `.flatten(0.1)`; group into an order-preserving `Vec<(Option<u32>, Vec<PlannedShape>)>` keyed by full stroke RGBA where alpha-0/None counts as skipped. `Cargo.toml`: deps `document`, `geometry`, `serde`(derive) — dev-dep `fontdb` for the font-picking test helper (copy the 5-line `any_available_family` used in document tests).
- [ ] **Step 4: Run to verify pass.** `cargo test -p cutplan -p document` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/ && git commit -m "Add cutplan crate with color-pass planning and travel moves"`

---

### Task 10: cutplan — preflight validation

**Files:**
- Create: `crates/cutplan/src/preflight.rs` (+ `pub mod preflight;`)

**Interfaces:**
- Consumes: `PlannedCut`, `driver_core::{MachineProfile, MachineCaps, Settings}`.
- Produces:

```rust
pub struct ConfiguredPass<'a> { pub pass: &'a ColorPass, pub settings: Settings, pub enabled: bool }
#[derive(Debug, PartialEq)]
pub enum PreflightError {
    NothingToCut, NonFiniteGeometry(NodeId), DegeneratePolyline(NodeId),
    OutOfBounds { node: NodeId, bounds: (f64, f64, f64, f64) },
    SettingsOutOfRange(&'static str), MachineMismatch { document: String, device: String },
    OutputTooLarge(usize),
}
pub fn preflight(passes: &[ConfiguredPass], profile: &MachineProfile, caps: &MachineCaps,
                 doc_machine_id: Option<&str>, allow_out_of_bounds: bool) -> Result<(), PreflightError>;
```

Rules (each its own test): all enabled passes empty → `NothingToCut`; any NaN/inf coordinate → `NonFiniteGeometry`; polyline `< 2` points → `DegeneratePolyline`; geometry outside `0..width_mm × 0..height_mm` → `OutOfBounds` unless `allow_out_of_bounds`; `repeat_count` outside `1..=10` or (when caps support them) speed outside `1..=30` / force outside `1..=33` (Cameo tables from `docs/protocol/silhouette-cameo5.md` — read them for exact bounds and cite) → `SettingsOutOfRange`; `doc_machine_id` set and ≠ `profile.id` → `MachineMismatch`; estimated encoded size > 64 MB → `OutputTooLarge` (estimate: 16 bytes/point × repeat_count, documented as an estimate).

- [ ] **Step 1: Write the failing tests** — one per rule above, plus a passing happy path.
- [ ] **Step 2: Run to verify failure.** `cargo test -p cutplan preflight::` → FAIL.
- [ ] **Step 3: Implement** as straightforward checks in rule order (first violation wins), no allocation-heavy passes.
- [ ] **Step 4: Run to verify pass.** `cargo test -p cutplan` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/cutplan/ && git commit -m "Add cut preflight validation with typed rejection reasons"`

---

### Task 11: cutplan — material presets

**Files:**
- Create: `crates/cutplan/src/presets.rs` (+ `pub mod presets;`)
- Modify: `crates/cutplan/Cargo.toml` (add `serde_json`, `tempfile`, `dirs`)

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialPreset { pub id: String, pub name: String, pub machine_id: String,
                            pub settings: PresetSettings, pub builtin: bool }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresetSettings { pub speed: Option<u32>, pub force: Option<u32>, pub repeat_count: u32 }
pub fn builtin_presets() -> Vec<MaterialPreset>;             // ≥4 per machine, cited sources
pub fn load_presets(user_file: &std::path::Path) -> Result<Vec<MaterialPreset>, PresetError>; // builtins + shadowing merge
pub fn save_user_presets(user_file: &std::path::Path, user: &[MaterialPreset]) -> Result<(), PresetError>; // atomic, versioned
#[derive(Debug, PartialEq)]
pub enum PresetError { Corrupt(String), UnknownVersion(u32), Io(String) }
// file format: { "version": 1, "presets": [ ...non-builtin entries... ] }
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn user_entry_shadows_builtin_and_delete_reveals_it() { /* same id overrides; removing user entry → builtin back */ }
#[test] fn corrupt_and_unknown_version_files_error_without_clobbering() {
    // write garbage → load = Err(Corrupt); file content unchanged on disk
    // {"version": 99} → Err(UnknownVersion(99))
}
#[test] fn save_is_atomic_and_round_trips() { /* save → load merge contains entry; tempdir */ }
#[test] fn builtins_cover_both_machines_with_valid_ranges() {
    // every builtin: machine_id ∈ {cameo5, puma}; repeat_count 1..=10; puma presets have speed/force None (panel-set)
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p cutplan presets::` → FAIL.
- [ ] **Step 3: Implement.** Builtins: cameo5 — cardstock-medium (speed 5, force 20), vinyl-adhesive (8, 10), htv (8, 12), copy-paper (10, 8), cardboard-thin (3, 30); puma — same materials, `speed/force: None`, `repeat_count 1` (values from inkscape-silhouette defaults; add the citation line to `docs/protocol/README.md`). Save via `tempfile::NamedTempFile` + `persist` (same pattern as `fileio::save_project`).
- [ ] **Step 4: Run to verify pass.** `cargo test -p cutplan` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/cutplan/ docs/protocol/ && git commit -m "Add builtin and user material presets with atomic versioned storage"`

---

### Task 12: desktop — factory, DeviceManagerHandle, IPC, event bridge, lifecycle

**Files:**
- Create: `apps/desktop/src/device.rs` (factory + handle + bridge)
- Modify: `apps/desktop/src/{lib.rs,main.rs,ipc.rs,state.rs}`, `apps/desktop/Cargo.toml` (add `cutplan`, `driver-silhouette`, `driver-hpgl` deps)

**Interfaces:**
- Consumes: everything above.
- Produces IPC commands (snake_case, new-style errors):

```rust
#[derive(Serialize)] pub struct IpcError { pub code: String, pub message: String }
// commands: list_devices, connect_device(info), disconnect_device, get_device_state,
// plan_cut() -> {passes:[{color, shape_count, node_ids}], skipped_no_stroke, doc_revision, travel:[[x1,y1,x2,y2]]},
// cut(CutRequest), cancel_cut, resume_cut, confirm_pass_done,
// list_presets(machine_id), save_preset(p), delete_preset(id)
#[derive(Deserialize)] pub struct CutRequest { pub device_instance_id: String, pub doc_revision: u64,
    pub passes: Vec<ConfiguredPassDto> }
#[derive(Deserialize)] pub struct ConfiguredPassDto { pub color: Option<u32>, pub enabled: bool,
    pub preset_id: Option<String>, pub speed: Option<u32>, pub force: Option<u32>, pub repeat_count: Option<u32> }
```

- `DesktopBackendFactory` mirrors the CLI factory (Task 5). `DeviceManagerHandle` is separate Tauri managed state (`Arc<DeviceManager>` + `Mutex<Option<DeviceInfo>>` for the connected info) — device commands never lock the document mutex.
- Event bridge thread: sole receiver → `app.emit("device-event", payload)`; coalesce `Progress` to ≤10 Hz (drop intermediate); dropped webview listeners ignored.
- `cut` handler: locks doc briefly to `plan_passes` + revalidate `doc_revision` (mismatch → `IpcError{code:"stale_plan"}`); resolves presets/overrides into `Settings`; runs `preflight`; builds `Vec<CutPass>` in configured order; calls `manager.cut`.
- Lifecycle: `on_window_event` close-requested while state is active → emit a `cut-in-progress` event and prevent close until UI confirms (`force_quit` command cancels + shuts down + exits); normal exit path calls `manager.shutdown()`.

- [ ] **Step 1: Write the failing tests** (state-layer, no Tauri runtime — same pattern as SP3's `state.rs` tests)

```rust
#[test]
fn cut_request_with_stale_revision_is_rejected() {
    let mut app = AppState::new();
    let dev = test_device_setup();                  // handle over test factory + mock transport
    app.add_rect(10.0, 10.0);
    let plan = plan_for(&app);
    app.add_rect(5.0, 5.0);                          // mutate after planning
    let err = dev.cut_from_request(&app, request_from(plan)).unwrap_err();
    assert_eq!(err.code, "stale_plan");
}
#[test]
fn preset_and_override_resolution_prefers_override() { /* preset force 20, override force 25 → Settings.force 25 */ }
#[test]
fn preflight_failures_map_to_ipc_codes() { /* empty request → code "nothing_to_cut" */ }
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p desktop` → FAIL.
- [ ] **Step 3: Implement** per interfaces; `ipc.rs` gains thin `#[tauri::command]` wrappers (device ones over the handle, not the doc mutex); `main.rs` manages both states, spawns the bridge, registers handlers, wires lifecycle.
- [ ] **Step 4: Run to verify pass.** `cargo test -p desktop && cargo check -p desktop --all-targets` → clean, zero warnings.
- [ ] **Step 5: Commit.** `git add apps/desktop/ && git commit -m "Wire device manager, cut planning, and presets through Tauri IPC"`

---

### Task 13: UI — cut dialog view-model (pure logic, TDD)

**Files:**
- Create: `apps/desktop/ui/src/cut/viewmodel.ts`, `apps/desktop/ui/src/cut/viewmodel.test.ts`

**Interfaces:**
- Produces (Task 14 wires these):

```ts
export type PassVm = { color: number | null; shapeCount: number; enabled: boolean;
                       presetId: string | null; speed: number | null; force: number | null; repeatCount: number | null };
export type Caps = { supportsSpeed: boolean; supportsForce: boolean; needsOperatorPassConfirm: boolean };
export function reorderPass(passes: PassVm[], index: number, dir: -1 | 1): PassVm[];       // pure, clamps at ends
export function effectiveSettings(p: PassVm, presets: Preset[]): { speed: number | null; force: number | null; repeatCount: number };
export function fieldDisabled(field: "speed" | "force", caps: Caps): boolean;
export function acceptEvent(currentJobId: number | null, ev: { job_id: number }): boolean; // stale-event filter
export function toCutRequest(deviceInstanceId: string, docRevision: number, passes: PassVm[]): CutRequest;
```

- [ ] **Step 1: Write the failing tests** — reorder clamps + swaps; override beats preset, preset beats default repeatCount 1; `fieldDisabled("speed", pumaCaps) === true`; `acceptEvent(2, {job_id: 1}) === false`; `toCutRequest` serializes only what the Rust `ConfiguredPassDto` expects (snake_case keys via the wrapper, camelCase args at invoke boundary as established).
- [ ] **Step 2: Run to verify failure.** `cd apps/desktop/ui && npx vitest run viewmodel` → FAIL.
- [ ] **Step 3: Implement** the pure functions.
- [ ] **Step 4: Run to verify pass.** `npx vitest run viewmodel` then full `npm test` → PASS.
- [ ] **Step 5: Commit.** `git add apps/desktop/ui/src/cut/ && git commit -m "Add cut dialog view-model logic"`

---

### Task 14: UI — cut dialog, preview canvas, event wiring

**Files:**
- Create: `apps/desktop/ui/src/cut/CutDialog.tsx`, `apps/desktop/ui/src/cut/CutPreview.tsx`
- Modify: `apps/desktop/ui/src/ipc.ts` (new wrappers), `apps/desktop/ui/src/App.tsx` (Cut button + dialog mount + device-event listener), `apps/desktop/ui/src/panels/TopBar.tsx` (Cut button, aria "Cut"), `apps/desktop/ui/src/panels/StatusBar.tsx` (state-driven dot)

**Interfaces:**
- Consumes: Task 12 IPC + `device-event` channel (`@tauri-apps/api/event.listen`), Task 13 view-model.
- Produces: working dialog per spec §5 — device picker (candidates labeled "unverified serial device"), pass rows (swatch/count/enable/preset/overrides with caps-disabled fields + "set on the Puma's panel" hint/order buttons), "Not cut: N shapes" line, Cut/Cancel/Resume/Confirm-pass-done buttons by state, bytes-based progress labeled "sending", errors in `--cut`, stale-plan error prompts replan, mismatch blocks Cut with convert prompt, preview (artboard outline, pass-colored paths, order badges, dashed travel lines re-rendered on config change).

- [ ] **Step 1: Playwright-first.** Extend `e2e/smoke.spec.ts` mock: handlers for the new commands (`plan_cut` returns a 2-pass plan from the mock doc's strokes; `cut` emits scripted `device-event`s through the mock — `__TAURI_INTERNALS__` event emulation: store listeners registered via the events API and invoke them), plus a new spec:

```ts
test("two-color doc cuts through swap and resume", async ({ page }) => {
  // seed two rects with different strokes via mock add handlers, open Cut dialog,
  // expect 2 pass rows, click Cut, expect "waiting for color swap", click Resume,
  // expect completion state, dialog closable
});
```

- [ ] **Step 2: Run to verify failure.** `npm run e2e` → new spec FAILS (no dialog).
- [ ] **Step 3: Implement** components + wiring until the new spec and the old smoke both pass. Keep components thin over the view-model; preview draws with the same 1px=1mm mapping as the editor canvas.
- [ ] **Step 4: Verify.** `npm test && npm run build && npm run e2e` all green; rebuild `dist/` and include it.
- [ ] **Step 5: Commit.** `git add apps/desktop/ui/ && git commit -m "Add cut dialog with pass preview and device event wiring"`

---

### Task 15: CLI — `cut --by-color`

**Files:**
- Modify: `crates/cli/src/main.rs`, `crates/cli/src/pipeline.rs`, `crates/cli/Cargo.toml` (add `cutplan`, `fileio`, `document` deps)

**Interfaces:**
- Produces: `cuthulhu cut file.svg --device cameo5 --by-color [--skip-color RRGGBBAA]... [--order RRGGBBAA,RRGGBBAA,...]`; imports SVG → `Document` (via `fileio::import_svg` into a fresh doc), `plan_passes`, per-pass sending through `DeviceManager`; between passes prints `Pass 2/3 (color #0000ff): swap tool, press Enter to resume` and waits for Enter on a TTY; non-TTY multi-color exits with `error: --by-color requires an interactive terminal` (exit code 2); Ctrl-C triggers `cancel()` (ctrlc handler or check on the Enter read) and prints the cancelled-pass summary. Single-color path byte-identical to today.

- [ ] **Step 1: Write the failing tests** (pipeline-level, no real device)

```rust
#[test]
fn by_color_plans_from_svg_and_respects_skip_and_order_flags() {
    let svg = two_color_svg();
    let plan = plan_from_svg(svg, &["ff0000ff".into()], Some("0000ffff,ff0000ff".into())).unwrap();
    assert_eq!(plan.len(), 1, "red skipped");            // order flag applied before skip filter
}
#[test]
fn noninteractive_multicolor_is_an_error() { /* is_tty=false + 2 passes → typed error */ }
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p cli` → FAIL.
- [ ] **Step 3: Implement**; TTY detection via `std::io::IsTerminal`.
- [ ] **Step 4: Run to verify pass.** `cargo test --workspace` → all pass, zero warnings.
- [ ] **Step 5: Commit.** `git add crates/cli/ && git commit -m "Add interactive multi-color cutting to the CLI"`

---

### Task 16: Integration verification + manual checklist

**Files:**
- Modify: `apps/desktop/MANUAL-CHECKLIST.md`

- [ ] **Step 1: Full verification.** `cargo test --workspace` (zero warnings), `cd apps/desktop/ui && npm test && npm run build && npm run e2e`.
- [ ] **Step 2: Real-app walk (mock device not possible — no hardware in the loop):** launch the app, import a two-color SVG (colors now real from Task 1), open the Cut dialog, verify: 2 passes with correct swatches, reorder works, preview shows order badges + travel lines, device list shows "no devices" gracefully (empty state, no error), Cut disabled without a connected device. Screenshot evidence.
- [ ] **Step 3: Update `MANUAL-CHECKLIST.md`** — add the SP4 hardware section verbatim from the spec's Testing list: Cameo (one prologue/epilogue on-device, moving→ready polling, safe park, registered two-color overlay, cancel behavior, unplug recovery — **these close SP4**) and Puma (non-blocking; "host-queue drain ≠ cutter completion" note).
- [ ] **Step 4: Commit.** `git add apps/desktop/MANUAL-CHECKLIST.md && git commit -m "Add SP4 hardware checklist"`

---

## Self-review

**Spec coverage:** prerequisites (Tasks 1–2) · session contract + caps + repeat_count (3) · transport read/write_all (4) · DeviceInfo/factory/instance-targeted open (5) · manager lifecycle/cut/cancel with completion policies, job ids, bounded channel, shutdown (6–8) · plan_passes/travel/doc_revision/shape_outline incl. Text (9) · preflight rules (10) · presets incl. corrupt/version/atomic/shadow (11) · IPC schemas, handle separation, bridge coalescing, lifecycle prompt (12) · view-model + dialog + preview + stale-event filtering (13–14) · CLI TTY semantics (15) · checklist hardware gates (16). Spec's out-of-scope list has no tasks — correct.

**Placeholder scan:** every code step has concrete code or an exact rule list; the two "read the existing trailer, preserve byte-for-byte" notes point at specific existing code with a byte-identity test enforcing correctness; test bodies abbreviated with comments name their exact assertions.

**Type consistency:** `MachineCaps`/`Settings.repeat_count`/session trait (3) used in 7–12; `TransportError::{Timeout,WriteZero}` (4) in 8; `DeviceInfo`/factory (5) in 6, 12; `DeviceState`/`DeviceEvent`/`CutPass` (6) in 7–8, 12; `PlannedCut`/`PlanError`/`doc_revision` (9) in 10, 12; `ConfiguredPass` (10) vs `ConfiguredPassDto` (12) distinct on purpose (borrowed engine type vs owned wire type); `PassVm`/`Caps` (13) in 14.
