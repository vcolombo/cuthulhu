# SP4 — Cut workflow: design spec

Date: 2026-07-24
Status: approved (brainstorming complete)
Parent: `2026-07-21-cuthulhu-design.md` (sub-project 4 of 6)

## Purpose

Connect the editor to the machines: pick a device, pick a material, preview what will cut, and cut it — including multi-color documents as ordered passes with per-color settings. First time the GUI drives the SP2 drivers.

## Decisions

- **Weed lines: deferred** to SP4.1 — they need polygon offsetting, a geometry capability that does not exist yet. Everything else from the roadmap line ships.
- **Event-push device communication** (not polling): the UI must reflect device state instantly; a poll loop during a 10-minute cut contradicts the performance pillar.
- **Presets: built-ins + user-editable**, stored in a documented open JSON format.
- **Cut-by-color: full** — per-color settings, ordering, and skip, with operator-confirmed pauses between colors.
- **Preview: static** — paths by pass, order badges, travel moves. No animated head simulation.
- **Hardware bar: real GUI-initiated multi-color cut on the Cameo 5** closes SP4. The Puma IV ships mock-verified (golden bytes); its hardware checklist item is tracked but not blocking.

## Architecture

New crate `cutplan` (editor→machine pipeline) + a `DeviceManager` added to `driver-core`. The Tauri layer stays a thin bridge. The CLI reuses `cutplan` for multi-pass cutting (`cuthulhu cut --by-color`), keeping the engine the product.

```
document ──► cutplan ──► driver-core ──► driver-{silhouette,hpgl}
             passes,      DeviceManager,      encode/abort bytes
             presets,     events, transport
             travel
apps/desktop: IPC commands + DeviceEvent→Tauri-event bridge (~thin)
```

Rejected alternatives: everything in `driver-core` (drags editor concepts into the device-abstraction crate other projects should reuse); everything in `apps/desktop` (Tauri-locked, headless-untestable, violates "the engine is the open-source product").

## Components

### 1. `driver-core::manager` — DeviceManager

State machine, owned by a worker thread:

```
Disconnected → Idle → Sending{pass_index, done_bytes, total_bytes}
                        → WaitingForColorSwap{next_pass_index} → Sending…
                        → Idle (job complete)
any state → Error(DeviceError)
```

- `DeviceError { Disconnected, Busy, Io(String) }` — typed, per the master spec. Jam/media sensors need protocol feedback SP2 did not capture; `Io` carries raw detail until a future capture adds variants.
- Events over `std::sync::mpsc::Sender<DeviceEvent>`: `StateChanged(DeviceState)`, `Progress{pass_index, done_bytes, total_bytes}`, `PassComplete(pass_index)`, `JobComplete`, `Failed(DeviceError)`. Tests consume the `Receiver` directly against `MockTransport` — no Tauri.
- Writes are chunked (~4 KB) so progress is real and cancellation is prompt. `cancel()` sets an `AtomicBool` checked between chunks; on cancel, the driver's `abort_bytes()` (new optional `Driver` trait method — e.g. HPGL pen-up + return home; Silhouette equivalent from protocol notes) is written best-effort, then state → `Idle`.
- `resume()` advances `WaitingForColorSwap` → next pass; only valid in that state (`Busy` error otherwise).
- Enumeration: `list_devices() -> Vec<DeviceInfo>` re-scans USB (Silhouette VID/PIDs from SP2) and serial ports on demand. UI calls it on dialog open + a manual Refresh button. No background polling.
- `connect(device_info)` opens the transport and transitions `Disconnected → Idle` (or `Error` if opening fails); selecting a device in the picker triggers it. Cutting while `Disconnected` is a `Disconnected` error, never an implicit connect — connection state must be visible before bytes move.

### 2. `cutplan::presets` — material presets

```rust
pub struct MaterialPreset {
    pub id: String,          // "cardstock-medium" / user-chosen
    pub name: String,
    pub machine_id: String,  // matches driver_core::MachineProfile.id
    pub settings: driver_core::Settings,  // speed/force/passes
    pub builtin: bool,
}
```

- Built-in seed list compiled in as data, per machine: cardstock, adhesive vinyl, HTV, copy paper, thin cardboard. Values sourced from inkscape-silhouette/community tables, cited in `docs/protocol/` per the attribution rule.
- User file `presets.json` in the platform config directory (`dirs::config_dir()/cuthulhu/`), documented open format. Load = builtins + user entries, user entries shadow builtins by id. Save writes only user/modified entries.
- IPC CRUD: `list_presets(machine_id)`, `save_preset`, `delete_preset` (deleting a shadowed builtin restores the builtin). The cut dialog offers per-job override fields (speed/force/passes) that do not persist.

### 3. `cutplan::passes` — cut-by-color planning

```rust
pub struct ColorPass { pub color: Option<u32>, pub node_ids: Vec<NodeId>, pub polylines: Vec<Polyline> }
pub fn plan_passes(doc: &Document) -> Vec<ColorPass>;
pub fn travel_moves(passes: &[ColorPass]) -> Vec<(Point, Point)>;
```

- Groups Shape nodes by **stroke** color (fill ignored — cutters cut outlines). `stroke: None` shapes form a single "no color" pass listed last by default.
- Geometry flattened through each node's world transform (SP3.1 `world_transform`) then `Path::flatten(0.1)` — mm in, mm out; device-unit conversion stays in the drivers.
- Pass execution order, enable/skip, and per-pass preset/override are dialog state, applied when building the `Job` list; `plan_passes` itself is pure and order-stable (first-seen color order).
- `travel_moves` returns head-travel segments between consecutive polyline endpoints in final cut order — pure, for the preview only.
- Between passes the manager enters `WaitingForColorSwap`; the operator swaps tool/material and confirms Resume. Single-pass jobs never enter the wait state.

### 4. Cut dialog + preview (UI)

- Modal dialog over the editor, Escape-dismissable while idle (`Sending` requires Cancel first). Opened from a TopBar "Cut" button.
- Left column: device picker (`list_devices` + Refresh), pass list — one row per `ColorPass`: color swatch, shape count, enable checkbox, preset dropdown + override fields, order up/down buttons — and Cut / Cancel / Resume buttons that reflect manager state.
- Right column: preview canvas — artboard outline, each pass's paths stroked in its color, an order badge at each shape's start point, dashed travel-move segments in cut order.
- Progress: per-pass progress bar + state text driven by Tauri events (`@tauri-apps/api/event` listener), forwarded 1:1 from `DeviceEvent`s.
- Errors render in the dialog in `--cut` red; the StatusBar `ready` dot reflects manager state globally (green idle, cyan sending, red error).

### 5. IPC surface (added to the existing 15)

`list_devices`, `connect_device`, `cut(passes_config)`, `cancel_cut`, `resume_cut`, `get_device_state`, `list_presets`, `save_preset`, `delete_preset`, `plan_cut` (returns passes + travel moves for the preview). All `Result<T, String>` over typed errors, matching the established pattern.

## Error handling

No silent failures: worker-thread errors become `Failed(DeviceError)` events and surface in the dialog; every IPC returns a typed `Result`; cancel is always available during `Sending`; disconnect mid-job → `Error(Disconnected)` with the failed pass identified so the operator knows what was cut.

## Testing

- **Golden bytes** per driver: multi-pass job (settings change between passes), abort sequence — byte-exact files like SP2's.
- **DeviceManager state machine** over `MockTransport`: happy path (2-pass job with swap), mid-send cancel, error injection (transport failure mid-chunk), resume-in-wrong-state rejection. Assertions on the event stream.
- **`plan_passes` / `travel_moves`** unit tests: color grouping incl. nested-group world transforms, stroke-None pass, order stability.
- **UI**: vitest for dialog view-model logic (pass reordering, override merging); Playwright smoke extended — open cut dialog on a 2-color doc → 2 passes listed → mocked send runs to completion through a simulated color-swap resume.
- **Hardware (closes SP4)**: manual checklist — GUI multi-color cut on the Cameo 5. Puma IV item tracked, non-blocking.

## Out of scope (SP4)

- Weed lines (SP4.1, needs polygon offsets)
- Jam/media-sensor states beyond `Io` (needs USB capture)
- Print & cut registration (SP6)
- Animated toolpath preview
- Editor grouping UI (still SP5-adjacent; `plan_passes` handles nested groups defensively via world transforms)
