// SPDX-License-Identifier: GPL-3.0-or-later
use std::sync::{Arc, Mutex};

use cutplan::preflight::{preflight, ConfiguredPass, PreflightError};
use cutplan::presets::{default_presets_path, load_presets, save_user_presets, MaterialPreset};
use cutplan::{plan_passes, ColorPass};
use driver_core::manager::{CutPass, DeviceEvent, DeviceEventKind, DeviceManager, DeviceState};
use driver_core::{DeviceBackendFactory, DeviceInfo, Driver, Job, Settings, Transport, TransportError, TransportKind};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct IpcError { pub code: String, pub message: String }

impl IpcError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        IpcError { code: code.into(), message: message.into() }
    }
}

#[derive(Deserialize)]
pub struct CutRequest {
    pub device_instance_id: String,
    pub doc_revision: u64,
    pub passes: Vec<ConfiguredPassDto>,
}

#[derive(Deserialize)]
pub struct ConfiguredPassDto {
    pub color: Option<u32>,
    pub enabled: bool,
    pub preset_id: Option<String>,
    pub speed: Option<u32>,
    pub force: Option<u32>,
    pub repeat_count: Option<u32>,
}

/// Mirrors `cli::pipeline::CliBackendFactory` (Task 5) — same driver/transport
/// resolution, duplicated rather than shared since the CLI factory lives in a
/// binary crate `desktop` can't depend on.
pub struct DesktopBackendFactory;

impl DeviceBackendFactory for DesktopBackendFactory {
    fn list_devices(&self) -> Vec<DeviceInfo> {
        let mut devices: Vec<DeviceInfo> = driver_silhouette::list_locators()
            .into_iter()
            .map(|locator| DeviceInfo {
                instance_id: format!("usb:{locator}"),
                machine_id: "cameo5".into(),
                transport: TransportKind::Usb { locator },
                candidate: false,
            })
            .collect();
        devices.extend(driver_hpgl::list_ports().into_iter().map(|path| DeviceInfo {
            instance_id: format!("serial:{path}"),
            machine_id: "puma".into(),
            transport: TransportKind::Serial { path, baud: 9600 },
            candidate: true,
        }));
        devices
    }

    fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
        match machine_id {
            "cameo5" => Some(Box::new(driver_silhouette::SilhouetteDriver::new())),
            "puma" => Some(Box::new(driver_hpgl::HpglDriver::new())),
            _ => None,
        }
    }

    fn open_transport(&self, info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
        match &info.transport {
            TransportKind::Usb { locator } => Ok(Box::new(driver_silhouette::UsbTransport::open_at(locator)?)),
            TransportKind::Serial { path, baud } => Ok(Box::new(driver_hpgl::SerialTransport::open(path, *baud)?)),
        }
    }
}

/// Separate Tauri managed state from `AppStateHandle` — device commands go
/// through here and never touch the document mutex.
pub struct DeviceManagerHandle {
    factory: Arc<dyn DeviceBackendFactory>,
    // ponytail: brief says `Arc<DeviceManager>`; `DeviceManager::shutdown(self)`
    // consumes by value, so the Arc is wrapped in Option to let `shutdown()`
    // `.take()` it out and `Arc::try_unwrap` it. Upgrade if a design ever needs
    // several live handles to the same manager (not the case here — one per app).
    manager: Mutex<Option<Arc<DeviceManager>>>,
    pub connected: Mutex<Option<DeviceInfo>>,
    // ponytail: event-driven cache, kept current by the bridge thread calling
    // `record_state` for every event it forwards. Never blocks on the worker
    // thread (unlike `DeviceManager::snapshot()`), so it's safe to read from
    // the close handler or a polling command even mid-transmit; may lag the
    // true state by one event. Upgrade to a blocking read only if a caller
    // ever needs the exact instantaneous state (tests use `DeviceManager::
    // snapshot()` directly for that).
    state_cache: Mutex<DeviceState>,
}

impl DeviceManagerHandle {
    pub fn new(factory: Arc<dyn DeviceBackendFactory>) -> (Self, std::sync::mpsc::Receiver<DeviceEvent>) {
        let (mgr, events) = DeviceManager::spawn(factory.clone());
        let handle = DeviceManagerHandle {
            factory,
            manager: Mutex::new(Some(Arc::new(mgr))),
            connected: Mutex::new(None),
            state_cache: Mutex::new(DeviceState::Disconnected),
        };
        (handle, events)
    }

    fn manager(&self) -> Result<Arc<DeviceManager>, IpcError> {
        self.manager.lock().unwrap().clone()
            .ok_or_else(|| IpcError::new("shut_down", "device manager has been shut down"))
    }

    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        self.factory.list_devices()
    }

    pub fn connect(&self, info: DeviceInfo) -> Result<(), IpcError> {
        self.manager()?.connect(info.clone()).map_err(|e| IpcError::new("device_error", format!("{e:?}")))?;
        *self.connected.lock().unwrap() = Some(info);
        Ok(())
    }

    pub fn disconnect(&self) -> Result<(), IpcError> {
        self.manager()?.disconnect().map_err(|e| IpcError::new("device_error", format!("{e:?}")))?;
        *self.connected.lock().unwrap() = None;
        Ok(())
    }

    /// Called by the event bridge thread for every event it forwards; updates
    /// the cache on `StateChanged`. Also synthesizes `Transmitting` from
    /// `Progress` — the worker sets `Transmitting` directly and only emits
    /// `Progress` ticks while it holds, so without this arm the cache never
    /// observes an in-flight transmit and `is_active` stays stuck. Every
    /// other event kind is ignored.
    pub fn record_state(&self, event: &DeviceEvent) {
        match &event.kind {
            DeviceEventKind::StateChanged(s) => *self.state_cache.lock().unwrap() = s.clone(),
            DeviceEventKind::Progress { pass_index, submitted_bytes, total_bytes } => {
                *self.state_cache.lock().unwrap() = DeviceState::Transmitting {
                    job_id: event.job_id,
                    pass_index: *pass_index,
                    submitted_bytes: *submitted_bytes,
                    total_bytes: *total_bytes,
                };
            }
            _ => {}
        }
    }

    /// Non-blocking, event-driven snapshot — see `state_cache`'s doc comment.
    pub fn cached_state(&self) -> DeviceState {
        self.state_cache.lock().unwrap().clone()
    }

    pub fn cancel(&self) -> Result<(), IpcError> {
        self.manager()?.cancel();
        Ok(())
    }

    pub fn resume(&self) -> Result<(), IpcError> {
        self.manager()?.resume().map_err(|e| IpcError::new("device_error", format!("{e:?}")))
    }

    pub fn confirm_pass_done(&self) -> Result<(), IpcError> {
        self.manager()?.confirm_pass_done().map_err(|e| IpcError::new("device_error", format!("{e:?}")))
    }

    /// Normal-exit lifecycle path: take the sole stored `Arc`, unwrap it, and
    /// consume `DeviceManager::shutdown(self)`. If a call is mid-flight (a
    /// clone is briefly alive), that's a non-fatal race at process exit — log
    /// and move on rather than block or panic.
    pub fn shutdown(&self) {
        let Some(arc) = self.manager.lock().unwrap().take() else { return };
        match Arc::try_unwrap(arc) {
            Ok(mgr) => mgr.shutdown(),
            Err(_) => eprintln!("device manager shutdown skipped: a call was still in flight"),
        }
    }

    /// Locks `app`'s document just long enough to plan + revalidate, resolve
    /// presets/overrides, and run preflight — returns an owned `Vec<CutPass>`
    /// with no remaining borrow of `app`, so the caller drops the document
    /// lock *before* calling `execute_cut` (which blocks on the worker
    /// thread). Never touches `AppStateHandle`'s mutex beyond that.
    pub fn prepare_cut(&self, app: &AppState, request: CutRequest) -> Result<Vec<CutPass>, IpcError> {
        let planned = plan_passes(&app.editor.doc)
            .map_err(|e| IpcError::new("plan_error", format!("{e:?}")))?;
        if planned.doc_revision != request.doc_revision {
            return Err(IpcError::new("stale_plan", "document changed since the cut was planned"));
        }

        let connected = self.connected.lock().unwrap().clone()
            .ok_or_else(|| IpcError::new("not_connected", "no device connected"))?;
        if connected.instance_id != request.device_instance_id {
            return Err(IpcError::new("device_mismatch", "connected device changed since planning"));
        }

        let driver = self.factory.driver_for(&connected.machine_id)
            .ok_or_else(|| IpcError::new("unknown_machine", format!("no driver for `{}`", connected.machine_id)))?;
        let profile = driver.profile().clone();
        let caps = driver.caps();

        let presets: Vec<MaterialPreset> = if request.passes.iter().any(|p| p.preset_id.is_some()) {
            let path = default_presets_path()
                .ok_or_else(|| IpcError::new("no_config_dir", "cannot resolve presets file location"))?;
            load_presets(&path).map_err(|e| IpcError::new("preset_error", format!("{e:?}")))?
        } else {
            Vec::new()
        };

        let mut configured: Vec<ConfiguredPass> = Vec::with_capacity(request.passes.len());
        for dto in &request.passes {
            let pass = planned.passes.iter().find(|p| p.color == dto.color).ok_or_else(|| {
                IpcError::new("unknown_pass_color", format!("no planned pass matches color {:?}", dto.color))
            })?;
            let preset = dto.preset_id.as_deref().and_then(|id| presets.iter().find(|p| p.id == id));
            configured.push(ConfiguredPass { pass, settings: resolve_settings(preset, dto), enabled: dto.enabled });
        }

        let doc_machine_id = app.editor.doc.machine.as_ref().map(|m| m.id.as_str());
        preflight(&configured, &profile, &caps, doc_machine_id, false).map_err(map_preflight_error)?;

        Ok(configured.iter().filter(|c| c.enabled).map(|c| CutPass {
            job: Job {
                polylines: c.pass.shapes.iter().flat_map(|s| s.polylines.iter().cloned()).collect(),
                settings: c.settings.clone(),
            },
        }).collect())
    }

    /// Submits already-planned passes to the device manager. Blocks until the
    /// worker reaches its first pause point or completion — call this off the
    /// document lock (see `prepare_cut`) and from an async command so it
    /// doesn't freeze the Tauri main loop.
    pub fn execute_cut(&self, passes: Vec<CutPass>) -> Result<u64, IpcError> {
        self.manager()?.cut(passes).map_err(|e| IpcError::new("device_error", format!("{e:?}")))
    }

    /// Test convenience: `prepare_cut` + `execute_cut` in one call. Production
    /// callers (`ipc::cut`) keep the two steps separate so the document lock
    /// is dropped before the blocking `execute_cut` call.
    #[cfg(test)]
    fn cut_from_request(&self, app: &AppState, request: CutRequest) -> Result<u64, IpcError> {
        let passes = self.prepare_cut(app, request)?;
        self.execute_cut(passes)
    }
}

/// Override fields win over the resolved preset's; missing both falls back
/// to `Settings::default()` (repeat_count 1).
fn resolve_settings(preset: Option<&MaterialPreset>, dto: &ConfiguredPassDto) -> Settings {
    Settings {
        speed: dto.speed.or_else(|| preset.and_then(|p| p.settings.speed)),
        force: dto.force.or_else(|| preset.and_then(|p| p.settings.force)),
        repeat_count: dto.repeat_count.or_else(|| preset.map(|p| p.settings.repeat_count)).unwrap_or(1),
    }
}

/// True while a cut job is mid-flight — used by the window close handler to
/// decide whether to block the close and ask the UI to confirm.
pub fn is_active(state: &DeviceState) -> bool {
    use DeviceState::*;
    matches!(state, Transmitting { .. } | AwaitingCompletion { .. } | WaitingForColorSwap { .. } | CancelRequested { .. } | Stopping { .. })
}

fn map_preflight_error(e: PreflightError) -> IpcError {
    match e {
        PreflightError::NothingToCut => IpcError::new("nothing_to_cut", "no enabled pass has any geometry"),
        PreflightError::NonFiniteGeometry(id) => IpcError::new("non_finite_geometry", format!("node {id:?} has non-finite coordinates")),
        PreflightError::DegeneratePolyline(id) => IpcError::new("degenerate_polyline", format!("node {id:?} has a polyline with < 2 points")),
        PreflightError::OutOfBounds { node, bounds } => IpcError::new("out_of_bounds", format!("node {node:?} outside {bounds:?}")),
        PreflightError::SettingsOutOfRange(msg) => IpcError::new("settings_out_of_range", msg),
        PreflightError::MachineMismatch { document, device } => IpcError::new("machine_mismatch", format!("document targets `{document}`, connected device is `{device}`")),
        PreflightError::OutputTooLarge(size) => IpcError::new("output_too_large", format!("estimated encoded size {size} bytes exceeds 64MB")),
    }
}

#[derive(Debug, Serialize)]
pub struct PlanCutResponse {
    pub passes: Vec<PlanCutPassSummary>,
    pub skipped_no_stroke: usize,
    pub doc_revision: u64,
    pub travel: Vec<[f64; 4]>,
}

#[derive(Debug, Serialize)]
pub struct PlanCutPassSummary {
    pub color: Option<u32>,
    pub shape_count: usize,
    pub node_ids: Vec<document::NodeId>,
}

/// Summarizes `plan_passes` output for the UI — not the raw `PlannedCut`
/// (which carries full flattened polylines the cut dialog doesn't need).
pub fn plan_cut_response(doc: &document::Document) -> Result<PlanCutResponse, IpcError> {
    let planned = plan_passes(doc).map_err(|e| IpcError::new("plan_error", format!("{e:?}")))?;
    let refs: Vec<&ColorPass> = planned.passes.iter().collect();
    let travel = cutplan::travel_moves(&refs);
    Ok(PlanCutResponse {
        passes: planned.passes.iter().map(|p| PlanCutPassSummary {
            color: p.color,
            shape_count: p.shapes.len(),
            node_ids: p.shapes.iter().map(|s| s.node_id).collect(),
        }).collect(),
        skipped_no_stroke: planned.skipped_no_stroke,
        doc_revision: planned.doc_revision,
        travel: travel.into_iter().map(|(a, b)| [a.x, a.y, b.x, b.y]).collect(),
    })
}

/// Re-derives the on-disk *user-only* preset list (builtins always shadow-load
/// with `builtin:false` forced onto user entries — see `cutplan::presets::load_presets`)
/// so `save_preset`/`delete_preset` round-trip through `save_user_presets` correctly
/// without ever writing a builtin back to disk.
fn user_presets_path() -> Result<std::path::PathBuf, IpcError> {
    default_presets_path().ok_or_else(|| IpcError::new("no_config_dir", "cannot resolve presets file location"))
}

pub fn list_presets(machine_id: &str) -> Result<Vec<MaterialPreset>, IpcError> {
    let path = user_presets_path()?;
    let all = load_presets(&path).map_err(|e| IpcError::new("preset_error", format!("{e:?}")))?;
    Ok(all.into_iter().filter(|p| p.machine_id == machine_id).collect())
}

pub fn save_preset(preset: MaterialPreset) -> Result<(), IpcError> {
    let path = user_presets_path()?;
    let mut user: Vec<MaterialPreset> = load_presets(&path)
        .map_err(|e| IpcError::new("preset_error", format!("{e:?}")))?
        .into_iter().filter(|p| !p.builtin).collect();
    user.retain(|p| p.id != preset.id);
    user.push(MaterialPreset { builtin: false, ..preset });
    save_user_presets(&path, &user).map_err(|e| IpcError::new("preset_error", format!("{e:?}")))
}

pub fn delete_preset(id: &str) -> Result<(), IpcError> {
    let path = user_presets_path()?;
    let mut user: Vec<MaterialPreset> = load_presets(&path)
        .map_err(|e| IpcError::new("preset_error", format!("{e:?}")))?
        .into_iter().filter(|p| !p.builtin).collect();
    user.retain(|p| p.id != id);
    save_user_presets(&path, &user).map_err(|e| IpcError::new("preset_error", format!("{e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cutplan::PlannedCut;
    use driver_core::{MachineCaps, MachineProfile};

    struct TestDriver { profile: MachineProfile, caps: MachineCaps }
    impl Driver for TestDriver {
        fn profile(&self) -> &MachineProfile { &self.profile }
        fn caps(&self) -> MachineCaps { self.caps }
        fn session_begin(&self) -> Vec<u8> { vec![] }
        fn encode_pass(&self, _pass: &Job) -> Result<Vec<u8>, driver_core::DriverError> { Ok(vec![]) }
        fn pass_park(&self) -> Vec<u8> { vec![] }
        fn session_end(&self) -> Vec<u8> { vec![] }
        fn abort_bytes(&self) -> Option<Vec<u8>> { None }
    }

    struct TestFactory;
    impl DeviceBackendFactory for TestFactory {
        fn list_devices(&self) -> Vec<DeviceInfo> { vec![test_instance()] }
        fn driver_for(&self, _machine_id: &str) -> Option<Box<dyn Driver + Send>> {
            Some(Box::new(TestDriver {
                profile: MachineProfile { id: "cameo5".into(), name: "Test Cameo".into(), width_mm: 500.0, height_mm: 500.0 },
                caps: MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: false },
            }))
        }
        fn open_transport(&self, _info: &DeviceInfo) -> Result<Box<dyn Transport>, TransportError> {
            Ok(Box::new(driver_core::MockTransport::default()))
        }
    }

    fn test_instance() -> DeviceInfo {
        DeviceInfo {
            instance_id: "usb:1:4".into(),
            machine_id: "cameo5".into(),
            transport: TransportKind::Usb { locator: "1:4".into() },
            candidate: false,
        }
    }

    fn test_device_setup() -> DeviceManagerHandle {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        dev.connect(test_instance()).unwrap();
        dev
    }

    fn plan_for(app: &AppState) -> PlannedCut {
        plan_passes(&app.editor.doc).unwrap()
    }

    fn request_from(plan: PlannedCut) -> CutRequest {
        CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: plan.doc_revision,
            passes: plan.passes.iter().map(|p| ConfiguredPassDto {
                color: p.color, enabled: true, preset_id: None, speed: None, force: None, repeat_count: None,
            }).collect(),
        }
    }

    #[test]
    fn cut_request_with_stale_revision_is_rejected() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        app.add_rect(5.0, 5.0);
        let err = dev.cut_from_request(&app, request_from(plan)).unwrap_err();
        assert_eq!(err.code, "stale_plan");
    }

    #[test]
    fn preset_and_override_resolution_prefers_override() {
        let preset = MaterialPreset {
            id: "p1".into(), name: "Test".into(), machine_id: "cameo5".into(),
            settings: cutplan::presets::PresetSettings { speed: Some(5), force: Some(20), repeat_count: 1 },
            builtin: false,
        };
        let dto = ConfiguredPassDto {
            color: Some(0x000000FF), enabled: true, preset_id: Some("p1".into()),
            speed: None, force: Some(25), repeat_count: None,
        };
        let settings = resolve_settings(Some(&preset), &dto);
        assert_eq!(settings.force, Some(25));
        assert_eq!(settings.speed, Some(5));
        assert_eq!(settings.repeat_count, 1);
    }

    #[test]
    fn preflight_failures_map_to_ipc_codes() {
        let app = AppState::new();
        let dev = test_device_setup();
        let revision = cutplan::doc_revision(&app.editor.doc);
        let request = CutRequest { device_instance_id: test_instance().instance_id, doc_revision: revision, passes: vec![] };
        let err = dev.cut_from_request(&app, request).unwrap_err();
        assert_eq!(err.code, "nothing_to_cut");
    }

    #[test]
    fn unknown_pass_color_is_rejected_not_dropped() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        let mut request = request_from(plan);
        request.passes[0].color = Some(0xDEADBEEF); // doesn't match any planned pass
        let err = dev.cut_from_request(&app, request).unwrap_err();
        assert_eq!(err.code, "unknown_pass_color");
    }

    #[test]
    fn progress_event_marks_cache_transmitting() {
        let dev = test_device_setup();
        assert!(!is_active(&dev.cached_state()));
        let event = DeviceEvent {
            job_id: 1,
            kind: DeviceEventKind::Progress { pass_index: 0, submitted_bytes: 10, total_bytes: 100 },
        };
        dev.record_state(&event);
        assert!(is_active(&dev.cached_state()));
    }
}
