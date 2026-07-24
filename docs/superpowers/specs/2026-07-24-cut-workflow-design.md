# SP4 — Cut workflow: design spec

Date: 2026-07-24
Status: approved (brainstorming complete; revised after external design review)
Parent: `2026-07-21-cuthulhu-design.md` (sub-project 4 of 6)

## Purpose

Connect the editor to the machines: pick a device, pick a material, preview what will cut, and cut it — including multi-color documents as ordered passes with per-color settings. First time the GUI drives the SP2 drivers.

## Decisions

- **Weed lines: deferred** to SP4.1 — they need polygon offsetting, a geometry capability that does not exist yet. Everything else from the roadmap line ships.
- **Event-push device communication** (not UI polling): the UI must reflect device state instantly. Internal protocol status polling (e.g. Cameo ENQ loops) is a driver concern and does not violate this.
- **Presets: built-ins + user-editable**, stored in a documented open JSON format.
- **Cut-by-color: full** — per-color settings, ordering, and skip, with operator-confirmed pauses between colors.
- **Preview: static** — paths by pass, order badges, travel moves. No animated head simulation.
- **Hardware bar: real GUI-initiated multi-color cut on the Cameo 5** closes SP4. The Puma IV ships mock-verified (golden bytes); its hardware checklist item is tracked but not blocking.

## Prerequisites (small, land first)

1. **SVG stroke-color preservation.** `fileio::paint_rgba` currently maps every paint to opaque black, and import converts missing strokes to black — an imported red/blue SVG becomes one black pass, making the multi-color hardware gate unreachable. Fix: map real resolved RGB (alpha from usvg opacity); preserve `stroke: None` instead of manufacturing black. Regression test: red/blue SVG imports as two distinct stroke colors. **`stroke: None` means "do not cut"** — such shapes are excluded from planning (listed in the dialog as "not cut: N shapes"). Pass grouping keys on full RGBA; fully transparent strokes (alpha 0) count as `None`.
2. **Canonical machine ids.** Document profiles say `cameo5_alpha`/`puma_iv`; driver profiles say `cameo5`/`puma`. Canonical ids become **`cameo5`, `puma`** everywhere (drivers already use them). `document::machine` migrates; project load maps legacy ids (`cameo5_alpha→cameo5`, `puma_iv→puma`) so existing saves open. Machine **model id** (`cameo5`) is distinct from **device instance id** (a specific plugged-in unit).

## Architecture

New crate `cutplan` (editor→machine pipeline) + a `DeviceManager` in `driver-core` that is **concrete-driver-free**: desktop and CLI inject a `DeviceBackendFactory` that constructs drivers/transports (the dependency direction stays drivers→core, apps→everything). The Tauri layer is a thin bridge. The CLI reuses `cutplan` for multi-pass cutting.

```
document ──► cutplan ──► driver-core ◄── DeviceBackendFactory (assembled in apps/desktop + cli)
             passes,      DeviceManager,          │
             presets,     session encoding,       ▼
             preflight,   events           driver-{silhouette,hpgl}
             travel
apps/desktop: IPC commands + DeviceEvent→Tauri-event bridge (thin)
```

Rejected alternatives: everything in `driver-core` (drags editor concepts into the reusable device crate); everything in `apps/desktop` (Tauri-locked, headless-untestable, violates "the engine is the open-source product").

## Components

### 1. Session-level encoding (driver contract change)

Today `Driver::encode(Job)` emits a complete session per call — Silhouette wraps every job in `ESC EOT` … `SO0`/`FN0` (feed-out/new-origin) and HPGL emits `IN;`. Calling it once per color pass would re-init and feed out between passes, moving media and destroying overlay registration. SP4 replaces per-job encoding with a session contract:

```rust
pub trait Driver {
    fn profile(&self) -> &MachineProfile;
    fn caps(&self) -> MachineCaps;
    fn session_begin(&self) -> Vec<u8>;                       // prologue, once
    fn encode_pass(&self, pass: &Job) -> Result<Vec<u8>, DriverError>;  // settings + geometry, no prologue/epilogue
    fn pass_park(&self) -> Vec<u8>;                           // pen-up/park between passes (documented sequence)
    fn session_end(&self) -> Vec<u8>;                         // epilogue/feed-out, once
    fn abort_bytes(&self) -> Option<Vec<u8>>;                 // best-effort stop payload; None until documented
}
```

- Overlay cutting keeps the same media clamped; between-pass swaps are **tool/blade only**. Replacing the material sheet is a separate one-pass-per-sheet workflow (out of SP4).
- The old single-job path (CLI `cut`) becomes `session_begin + encode_pass + session_end` — byte-identical for one pass; golden files updated accordingly.
- Silhouette `abort_bytes` stays `None` until a stop command is captured and documented (`docs/protocol/`); HPGL uses pen-up (queued, best-effort). Park sequences and overlay registration are hardware-checklist items.

### 2. `driver-core::manager` — DeviceManager

Worker-thread state machine; all concrete construction injected via `DeviceBackendFactory: Send + Sync`.

```
Disconnected → Connecting → Idle
Idle → Transmitting{job_id, pass_index, submitted_bytes, total_bytes}
     → AwaitingCompletion{pass_index}          // device confirms motion done (Cameo) or operator confirms (Puma)
     → WaitingForColorSwap{next_pass_index} → Transmitting…
     → Idle (after session_end)
Transmitting → CancelRequested → Stopping → Cancelled{pass_index, submitted_bytes, completion: Known|Unknown}
any active state → Error(DeviceError); Error → Idle via reconnect/clear; Disconnecting → Disconnected
```

- **`PassComplete` means the machine finished moving, not "bytes accepted".** `Transport` gains read support: `fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError>`. The Cameo backend polls ENQ/status on EP `0x82` (per `docs/protocol/silhouette-cameo5.md`) until `ready` before emitting `PassComplete`, with a timeout → `Error(Io)`. The Puma protocol notes document no completion query: its passes end in `AwaitingCompletion` requiring **operator confirmation** that motion stopped (UI button), recorded as such in the event.
- **Progress is defined as bytes submitted to the transport** — never physical cut progress. The UI labels it "sending".
- **Cancellation is cooperative and best-effort.** An `AtomicBool` stops future chunk writes; `abort_bytes` (if any) is written; but commands already buffered in USB/serial/device execute regardless. State reaches `Cancelled`/`Idle` only after device-ready confirmation (Cameo) or operator confirmation (Puma) — never while the machine may still be moving. Writes use bounded timeouts so a blocked transport cannot hang cancel forever.
- **Write contract:** transport writes become `write_all`-style: loop on partial writes, `Ok(0)` → `TransportError::WriteZero`. Chunk size ~4 KB.
- Events: `DeviceEvent { job_id: u64, kind: StateChanged(..) | Progress{..} | PassComplete(pass_index) | JobComplete | Cancelled{..} | Failed(DeviceError) }` over `mpsc`. Every event carries `job_id`; the UI drops events from stale jobs.
- Commands into the worker via a **bounded command channel with reply channels** (connect/cut/cancel/resume/disconnect/status-snapshot). The worker exclusively owns `Box<dyn Transport + Send>`. `resume()` valid only in `WaitingForColorSwap` (else `Busy`); `cut` while not `Idle` → `Busy`; `connect` while active → `Busy`.
- `DeviceError { Disconnected, Busy, Timeout, WriteZero, Io(String) }`. Unplug mid-anything → `Error(Disconnected)` naming the interrupted pass.
- Enumeration: `list_devices() -> Vec<DeviceInfo>`; `DeviceInfo { instance_id, machine_id, transport: Usb{locator}|Serial{path, baud}, candidate: bool }`. Serial ports cannot be assumed to be Pumas — they are listed as `candidate: true` requiring explicit user selection. USB open targets the **selected instance locator**, not the first VID/PID match. Rescan on dialog open + manual Refresh; no background polling.
- `MachineCaps { supports_speed: bool, supports_force: bool, needs_operator_pass_confirm: bool }` — drives preflight and UI (HPGL ignores speed/force: those fields disable with a "set on the Puma's panel" hint).

### 3. `cutplan::presets` — material presets

```rust
pub struct MaterialPreset {
    pub id: String,          // "cardstock-medium" / user-chosen
    pub name: String,
    pub machine_id: String,  // canonical model id: "cameo5" | "puma"
    pub settings: CutSettings,   // speed, force, repeat_count
    pub builtin: bool,
}
```

- `driver_core::Settings.passes` is renamed **`repeat_count`** (how many times each polyline is traced — not color passes). Validated `1..=10`; zero is rejected, never silently coerced.
- These are explicitly **speed/force/repeat presets**. Blade depth, tool selection, and mat settings are manual for SP4 (checklist instructions per material); extending Cameo tool commands is future work.
- Built-in seed list compiled in, per machine: cardstock, adhesive vinyl, HTV, copy paper, thin cardboard — values from inkscape-silhouette/community tables, cited in `docs/protocol/`.
- User file `presets.json` in `dirs::config_dir()/cuthulhu/` (documented format, versioned `{ "version": 1, "presets": [...] }`). Load = builtins + user entries; user entries shadow builtins by id; deleting a shadow restores the builtin. Corrupt/unknown-version files surface an error and load builtins only (never clobber the file silently); saves are atomic (temp + rename).
- IPC CRUD: `list_presets(machine_id)`, `save_preset`, `delete_preset`. Cut dialog offers per-job override fields that do not persist.

### 4. `cutplan::passes` — planning

```rust
pub struct PlannedShape { pub node_id: NodeId, pub polylines: Vec<Polyline> }
pub struct ColorPass { pub color: Option<u32>, pub shapes: Vec<PlannedShape> }
pub struct PlannedCut { pub passes: Vec<ColorPass>, pub skipped_no_stroke: usize, pub doc_revision: u64 }
pub enum PlanError { BadPath(NodeId, String), MissingNode(NodeId), CycleDetected }

pub fn plan_passes(doc: &Document) -> Result<PlannedCut, PlanError>;
pub fn travel_moves(configured: &[&ColorPass]) -> Vec<(Point, Point)>;  // final enabled order, post-dialog
```

- **Single preorder traversal** from `doc.root` carrying the accumulated transform (no per-node `world_transform` scans — that is quadratic on flat docs); detects missing children and cycles as typed errors. No silent failures: any unparsable path is a `PlanError`, not a dropped shape.
- One canonical shape-outline conversion, exposed from `document` (today's private `local_shape_path` becomes `pub`), extended to cover `ShapeKind::Text` via `text_to_path` — live text cuts as its outlines.
- Grouping keys on full stroke RGBA; `stroke: None`/alpha-0 shapes are counted in `skipped_no_stroke`, not cut. First-seen color order; order-stable.
- `doc_revision`: hash of the document snapshot at plan time. `cut` revalidates — a stale plan (document edited since preview) is rejected with a typed error prompting re-plan.
- `travel_moves` computes head-travel segments from the **configured** pass list (dialog's reorder/enable applied), not the initial plan.

### 5. Cut dialog + preview (UI)

- Modal over the editor, opened from a TopBar "Cut" button. Escape closes while idle; during an active job the dialog stays (Cancel first).
- Left: device picker (`list_devices` + Refresh; serial candidates labeled), pass list — swatch, shape count, enable, preset dropdown + override fields (fields disabled per `MachineCaps` with a panel hint for Puma), order up/down — and Cut / Cancel / Resume / Confirm-pass-done buttons reflecting manager state. "Not cut: N shapes" line when `skipped_no_stroke > 0`.
- Right: preview canvas — artboard outline, pass paths in pass color, order badges at shape start points, dashed travel segments in configured order. Re-renders on any config change.
- Progress bar ("sending", bytes-based) + state text from Tauri events; events carry `job_id` and stale-job events are ignored. Errors in `--cut` red; StatusBar `ready` dot reflects manager state globally.
- Document/device mismatch (connected model ≠ document machine) blocks Cut with an explicit convert-document prompt.

### 6. Preflight (in `cutplan`, runs before encoding)

Rejects with typed errors: non-finite coordinates; polylines with <2 points; empty/all-disabled jobs; geometry outside machine-bed bounds (explicit override flag allows clipped/out-of-bed cutting only when the user confirms); settings out of range (per-machine speed/force tables, `repeat_count 1..=10`); document/device model mismatch; oversized encoded output (sanity cap, e.g. 64 MB).

### 7. IPC + event schemas

Serializable types (all typed, JSON over Tauri):

```
CutRequest { device_instance_id, doc_revision, passes: Vec<ConfiguredPass> }
ConfiguredPass { color: Option<u32>, node_ids, preset_id: Option<String>, override: Option<CutSettings>, enabled }
IpcError { code: String, message: String }     // Result<T, IpcError> — replaces the Result<T, String> pattern for new commands
DeviceEvent { job_id, ...} on channel "device-event"
```

New IPC: `list_devices`, `connect_device`, `disconnect_device`, `plan_cut`, `cut(CutRequest)`, `cancel_cut`, `resume_cut`, `confirm_pass_done`, `get_device_state` (snapshot for dialog open), `list_presets`, `save_preset`, `delete_preset`. Existing 15 editor commands keep their current shape; new commands use `IpcError`.

### 8. Tauri integration & lifecycle

- Device state lives in its own managed `DeviceManagerHandle` (`Send + Sync`), **not** the document mutex — a long cut never blocks editor IPC.
- The event bridge owns the sole `Receiver`, forwards to the webview, coalesces `Progress` bursts (~10 Hz max), and treats a dropped webview listener as subscriber loss, not device failure.
- App exit: lifecycle hook sends cancel/shutdown, closes the transport, joins the worker (bounded wait). Quitting mid-cut prompts the user (cut in progress — cancel and quit / keep cutting).

### 9. CLI (`cuthulhu cut --by-color`)

SVG → temporary `Document` via the import path → `plan_passes`. Flags for pass order/skip/preset. Between passes: interactive TTY prompts "swap tool, Enter to resume"; Ctrl-C cancels (same cooperative semantics). Non-interactive (no TTY) multi-color runs error out with a message rather than hanging. Single-color behaves as today.

## Error handling

No silent failures: worker errors become `Failed` events naming the pass; every IPC returns a typed result; cancel is always available during `Transmitting`; disconnect mid-job → `Error(Disconnected)` with the interrupted pass identified; state never claims `Idle`/`Cancelled` while the machine may still be moving (completion confirmed by device status or operator).

## Testing

- **Golden bytes** per driver: session begin/pass/park/end framing — exactly one prologue and one epilogue across a 2-pass job; settings change between passes; single-pass output byte-identical to SP2's per-job encoding; abort payloads.
- **DeviceManager over `MockTransport`** (extended with scripted reads + failure injection): happy 2-pass with swap; mid-send cancel (incl. cancel during a blocked write via timeout); partial and zero writes; abort-write failure; encode failure; dropped event receiver; command-channel closure; double-cut/double-connect `Busy`; reconnect after `Error`; unplug in every active state; event ordering + `job_id` isolation across two jobs; shutdown/drop joins cleanly.
- **`plan_passes`/`travel_moves`**: every `ShapeKind` incl. Text; malformed path → typed error; no-stroke and transparent-stroke exclusion; RGBA grouping; multi-subpath shapes stay per-shape; missing/cyclic nodes; nested-group accumulated transforms; empty/all-skipped; preflight bounds and settings limits; `doc_revision` staleness rejection.
- **Presets**: corrupted/unknown-version JSON; duplicate ids; machine mismatch; invalid ranges; atomic-save recovery; delete-reveals-builtin.
- **UI**: vitest for dialog view-model (reorder, override merge, caps-driven field disabling, stale-event filtering); Playwright smoke extended — 2-color doc → 2 passes → mocked send through a simulated swap+resume to completion.
- **Hardware (Cameo 5, closes SP4)**: one prologue/one epilogue verified on-device; moving→ready status polling; safe park between passes; registered two-color overlay cut; cancel behavior observed; unplug mid-cut recovery. **Puma checklist** (non-blocking) explicitly records that host-queue drain ≠ cutter completion.

## Out of scope (SP4)

- Weed lines (SP4.1, needs polygon offsets)
- Jam/media-sensor states beyond `Io` (needs USB capture)
- Silhouette abort command (until captured + documented)
- Cameo blade-depth/tool/mat commands in presets (manual for now)
- Print & cut registration (SP6)
- Animated toolpath preview
- Editor grouping UI
