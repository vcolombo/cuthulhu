// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use cut_host::client::HostClient;
use cutplan::presets::{
    default_presets_path, load_presets, resolve_settings, save_user_presets, MaterialPreset,
    SettingsOverride,
};
use cutplan::{plan_cut, plan_passes, ColorPass, CutError, PassSelection, PlanOptions};
use driver_core::manager::{CutPass, DeviceEvent, DeviceManager};
use driver_core::{CutStatus, DeviceBackendFactory, DeviceInfo, HostId, MachineCaps};
use serde::{Deserialize, Serialize};

use crate::hosts::PairedHost;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct IpcError { pub code: String, pub message: String }

impl IpcError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        IpcError { code: code.into(), message: message.into() }
    }
}

#[derive(Deserialize)]
pub struct CutRequest {
    pub device_instance_id: String,
    pub doc_revision: String,
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

/// One paired Cut Host: what was saved about it, its connection if it has one, and why it has
/// none if it does not.
///
/// A host that is down keeps its entry so its cutters can be listed as unreachable rather than
/// disappearing — a cutter that vanishes looks like one that was never paired (#42).
pub(crate) struct HostConnection {
    pub paired: PairedHost,
    pub client: Option<HostClient>,
    pub last_error: Option<String>,
}

impl HostConnection {
    /// Connect if not already connected, and remember the reason if that fails.
    fn ensure(&mut self) -> Option<&HostClient> {
        if self.client.is_none() {
            match HostClient::connect(&self.paired.address, &self.paired.token, &self.paired.fingerprint) {
                Ok(c) => {
                    self.client = Some(c);
                    self.last_error = None;
                }
                Err(e) => self.last_error = Some(e.to_string()),
            }
        }
        self.client.as_ref()
    }
}

enum Route {
    Local,
    Host(HostId),
}

/// What the UI is told about a paired Cut Host.
///
/// Deliberately not `PairedHost`: that holds the token, and anything sent to the webview can
/// reach a `console.log` or a devtools session. The UI needs to render a row and address the
/// host by id; it does not need the secret.
#[derive(Clone, Debug, Serialize)]
pub struct PairedHostView {
    pub id: HostId,
    pub name: String,
    pub address: String,
    /// Why this host cannot be reached, or `None` when it can.
    pub unreachable: Option<String>,
}

/// How long `status()` will wait on a Cut Host before reporting disconnected. Short on purpose:
/// this is the window-close guard's own read, and it goes through the same `hosts` lock as every
/// other host call, so a poll that waited the full `DEFAULT_BODY_TIMEOUT` (30s) would make a quit
/// mid-cut look hung for half a minute.
const STATUS_POLL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Separate Tauri managed state from `AppStateHandle` — device commands go
/// through here and never touch the document mutex.
pub struct DeviceManagerHandle {
    local_factory: Arc<dyn DeviceBackendFactory>,
    // ponytail: brief said `Arc<DeviceManager>`; `DeviceManager::shutdown(self)` consumes by
    // value, so the Arc is wrapped in Option to let `shutdown()` take it out and unwrap it.
    local_manager: Mutex<Option<Arc<DeviceManager>>>,
    /// Every paired Cut Host, connected lazily. Held together rather than one-at-a-time
    /// because `list_devices` asks all of them and the device list shows them together.
    hosts: Mutex<HashMap<HostId, HostConnection>>,
    pub connected: Mutex<Option<DeviceInfo>>,
}

impl DeviceManagerHandle {
    pub fn new(factory: Arc<dyn DeviceBackendFactory>) -> (Self, std::sync::mpsc::Receiver<DeviceEvent>) {
        let (mgr, events) = DeviceManager::spawn(factory.clone());
        let handle = DeviceManagerHandle {
            local_factory: factory,
            local_manager: Mutex::new(Some(Arc::new(mgr))),
            hosts: Mutex::new(HashMap::new()),
            connected: Mutex::new(None),
        };
        (handle, events)
    }

    fn manager(&self) -> Result<Arc<DeviceManager>, IpcError> {
        self.local_manager.lock().unwrap().clone()
            .ok_or_else(|| IpcError::new("shut_down", "device manager has been shut down"))
    }

    /// Local hardware plus every paired Cut Host's cutters, in one list.
    ///
    /// A host that cannot be reached contributes nothing here and its reason shows up in
    /// `host_views` — the list is what can be cut on, not what has been configured.
    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        let mut all = self.local_factory.list_devices();
        let mut hosts = self.hosts.lock().unwrap();
        for (id, host) in hosts.iter_mut() {
            let Some(client) = host.ensure() else { continue };
            match client.devices() {
                Ok(devices) => all.extend(crate::hosts::stamp_host(id, devices)),
                Err(e) => {
                    // The connection went away between `ensure` and here; drop it so the next
                    // call reconnects rather than reusing a dead one.
                    host.last_error = Some(e.to_string());
                    host.client = None;
                }
            }
        }
        all
    }

    pub fn add_host(&self, paired: PairedHost) {
        let id = paired.id.clone();
        self.hosts
            .lock()
            .unwrap()
            .insert(id, HostConnection { paired, client: None, last_error: None });
    }

    pub fn remove_host(&self, id: &HostId) {
        self.hosts.lock().unwrap().remove(id);
    }

    /// What `list_hosts` gives the UI: enough to render a row and address it, never the token.
    pub(crate) fn host_views(&self) -> Vec<PairedHostView> {
        self.hosts
            .lock()
            .unwrap()
            .values()
            .map(|h| PairedHostView {
                id: h.paired.id.clone(),
                name: h.paired.name.clone(),
                address: h.paired.address.clone(),
                unreachable: h.last_error.clone(),
            })
            .collect()
    }

    /// Every paired host as saved, for `pair`/`forget` to re-derive `hosts.json`'s on-disk
    /// contents and to mint the next id against.
    pub(crate) fn paired_hosts(&self) -> Vec<PairedHost> {
        self.hosts.lock().unwrap().values().map(|h| h.paired.clone()).collect()
    }

    /// Pairs a Cut Host: prove it works, mint an id, persist — and only once the save has
    /// actually landed, hold it in memory.
    ///
    /// `hosts::save` is written atomic for exactly this class of failure (full disk, read-only
    /// config dir); ordering the in-memory insert after it means a failed save leaves disk and
    /// memory agreeing (host absent from both) instead of a phantom host live in the device list
    /// while the operator was told pairing failed.
    pub fn pair(
        &self,
        name: String,
        address: String,
        token: String,
        fingerprint: String,
        hosts_path: &Path,
    ) -> Result<HostId, IpcError> {
        // Prove it before writing it down, so a saved host has always worked at least once.
        HostClient::pair_check(&address, &token, &fingerprint)
            .map_err(|e| IpcError::new("host_unreachable", e.to_string()))?;

        let id = crate::hosts::next_id(&self.paired_hosts());
        let paired = PairedHost { id: id.clone(), name, address, fingerprint, token };

        let mut prospective = self.paired_hosts();
        prospective.push(paired.clone());
        crate::hosts::save(hosts_path, &prospective)
            .map_err(|e| IpcError::new("hosts_unwritable", e.to_string()))?;

        self.add_host(paired);
        Ok(id)
    }

    /// Forgets a Cut Host: refuses while any of its cutters is mid-cut, since the moment it is
    /// gone `cancel` routes to `unknown_host` (see `route`) — a blade still moving could no
    /// longer be stopped. Persist-then-mutate for the same reason as `pair`.
    pub fn forget(&self, id: &HostId, hosts_path: &Path) -> Result<(), IpcError> {
        // An unreachable host answers nothing, so it reports nothing as active — and there was
        // nothing `cancel` could have stopped there either. Only a host that answers and says
        // "busy" blocks the forget; already-unpaired and unreachable both fall through it.
        if let Ok(snapshots) = self.with_host(id, |c| c.snapshots()) {
            if snapshots.iter().any(|s| s.status.is_active()) {
                return Err(IpcError::new(
                    "host_busy",
                    "a cut is active on this host; cancel it before forgetting",
                ));
            }
        }

        let prospective: Vec<PairedHost> = self.paired_hosts().into_iter().filter(|h| &h.id != id).collect();
        crate::hosts::save(hosts_path, &prospective)
            .map_err(|e| IpcError::new("hosts_unwritable", e.to_string()))?;

        self.remove_host(id);
        Ok(())
    }

    /// Capability is the Driver's answer, not ours — the Driver that encodes the
    /// bytes is also what declares what they can carry.
    /// `Result`, not `Option`: an id the registry cannot build means the caller
    /// is out of sync with it, which is worth surfacing rather than defaulting.
    pub fn caps_for(&self, machine_id: &str) -> Result<MachineCaps, IpcError> {
        self.local_factory
            .driver_for(machine_id)
            .map(|d| d.caps())
            .ok_or_else(|| IpcError::new("unknown_machine", format!("no driver for `{machine_id}`")))
    }

    /// Where a call about `device` has to go. `None` is this computer; `Some(id)` is that host.
    ///
    /// An id nobody has paired is refused rather than falling back to local hardware: a Job
    /// aimed at a Pi must never be cut on the machine sitting on the desk.
    fn route(&self, device: &DeviceInfo) -> Result<Route, IpcError> {
        match &device.host {
            None => Ok(Route::Local),
            Some(id) if self.hosts.lock().unwrap().contains_key(id) => Ok(Route::Host(id.clone())),
            Some(id) => Err(IpcError::new("unknown_host", format!("no Cut Host called `{}` is paired", id.0))),
        }
    }

    /// Run `f` against the client for `id`, connecting if needed.
    fn with_host<T>(
        &self,
        id: &HostId,
        f: impl FnOnce(&HostClient) -> Result<T, cut_host::client::ClientError>,
    ) -> Result<T, IpcError> {
        let mut hosts = self.hosts.lock().unwrap();
        let host = hosts
            .get_mut(id)
            .ok_or_else(|| IpcError::new("unknown_host", format!("no Cut Host called `{}` is paired", id.0)))?;
        // Not `let client = host.ensure().ok_or_else(...)?;` — that binding would keep `host`'s
        // mutable borrow alive through the `None` arm's `host.last_error` read. Matching in
        // place lets the borrow end with the arm that doesn't need it.
        match host.ensure() {
            Some(client) => f(client).map_err(|e| IpcError::new("host_error", e.to_string())),
            None => Err(IpcError::new("host_unreachable", host.last_error.clone().unwrap_or_default())),
        }
    }

    pub fn connect(&self, info: DeviceInfo) -> Result<(), IpcError> {
        match self.route(&info)? {
            Route::Local => {
                self.manager()?
                    .connect(info.clone())
                    .map_err(|e| IpcError::new("device_error", format!("{e:?}")))?;
            }
            // A Cut Host connects each cutter itself at startup, so aiming at one is a local
            // bookkeeping act: there is no remote connection to open.
            Route::Host(_) => {}
        }
        *self.connected.lock().unwrap() = Some(info);
        Ok(())
    }

    pub fn disconnect(&self) -> Result<(), IpcError> {
        self.manager()?.disconnect().map_err(|e| IpcError::new("device_error", format!("{e:?}")))?;
        *self.connected.lock().unwrap() = None;
        Ok(())
    }

    /// Where the cut has got to. Reads `driver-core`'s published status, which
    /// never blocks on the worker — so the window-close handler and the IPC
    /// command can both call it freely, even mid-transmit.
    pub fn status(&self) -> CutStatus {
        let aimed = self.connected.lock().unwrap().clone();
        let Some(device) = aimed else { return CutStatus::disconnected() };
        match self.route(&device) {
            Ok(Route::Local) => match self.local_manager.lock().unwrap().as_ref() {
                Some(mgr) => mgr.status(),
                None => CutStatus::disconnected(),
            },
            // An unknown host is not a local cutter. Falling through to the local manager here
            // would report its `Idle` — and so `actions.cut` — for a device this desktop cannot
            // reach, offering a cut that `execute_cut` would then refuse.
            Err(_) => CutStatus::disconnected(),
            Ok(Route::Host(id)) => self
                // Bounded well under `DEFAULT_BODY_TIMEOUT` (30s): this is the window-close
                // guard's own read, and a stale snapshot is fine when the next poll is a second
                // away, but a 30s hold of `hosts` behind a stalled host is not.
                .with_host(&id, |c| c.snapshots_within(STATUS_POLL_TIMEOUT))
                .ok()
                .and_then(|snaps| {
                    snaps.into_iter().find(|s| s.info.instance_id == device.instance_id).map(|s| s.status)
                })
                // A host that cannot be reached mid-cut is not a finished cut: the Job is still
                // running on the Pi, and saying `Idle` here would invite a second dispatch.
                .unwrap_or_else(|| CutStatus::disconnected()),
        }
    }

    pub fn cancel(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => {
                self.manager()?.cancel();
                Ok(())
            }
            Some(Route::Host(id)) => {
                let device = aimed.expect("a route implies a device").instance_id;
                self.with_host(&id, |c| c.cancel(&device))
            }
        }
    }

    pub fn resume(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => {
                self.manager()?.resume().map_err(|e| IpcError::new("device_error", format!("{e:?}")))
            }
            Some(Route::Host(id)) => {
                let device = aimed.expect("a route implies a device").instance_id;
                self.with_host(&id, |c| c.resume(&device))
            }
        }
    }

    pub fn confirm_pass_done(&self) -> Result<(), IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => {
                self.manager()?.confirm_pass_done().map_err(|e| IpcError::new("device_error", format!("{e:?}")))
            }
            Some(Route::Host(id)) => {
                let device = aimed.expect("a route implies a device").instance_id;
                self.with_host(&id, |c| c.confirm_pass_done(&device))
            }
        }
    }

    /// Normal-exit lifecycle path: take the sole stored `Arc`, unwrap it, and
    /// consume `DeviceManager::shutdown(self)`. If a call is mid-flight (a
    /// clone is briefly alive), that's a non-fatal race at process exit — log
    /// and move on rather than block or panic.
    pub fn shutdown(&self) {
        let Some(arc) = self.local_manager.lock().unwrap().take() else { return };
        match Arc::try_unwrap(arc) {
            Ok(mgr) => mgr.shutdown(),
            Err(_) => eprintln!("device manager shutdown skipped: a call was still in flight"),
        }
    }

    /// Locks `app`'s document just long enough for `cutplan::plan_cut` to plan,
    /// revalidate and preflight it — returns an owned `Vec<CutPass>` with no
    /// remaining borrow of `app`, so the caller drops the document lock
    /// *before* calling `execute_cut` (which blocks on the worker thread).
    /// Never touches `AppStateHandle`'s mutex beyond that.
    ///
    /// What stays here is what `cutplan` cannot know: which device is plugged
    /// in, which driver serves it, and where the presets file lives.
    pub fn prepare_cut(&self, app: &AppState, request: CutRequest) -> Result<Vec<CutPass>, IpcError> {
        let connected = self.connected.lock().unwrap().clone()
            .ok_or_else(|| IpcError::new("not_connected", "no device connected"))?;
        if connected.instance_id != request.device_instance_id {
            return Err(IpcError::new("device_mismatch", "connected device changed since planning"));
        }

        let driver = self.local_factory.driver_for(&connected.machine_id)
            .ok_or_else(|| IpcError::new("unknown_machine", format!("no driver for `{}`", connected.machine_id)))?;
        let profile = driver.profile().clone();
        let caps = driver.caps();

        // Only enabled passes are cut, so only their presets are worth reading.
        let enabled = || request.passes.iter().filter(|p| p.enabled);
        let presets: Vec<MaterialPreset> = if enabled().any(|p| p.preset_id.is_some()) {
            let path = default_presets_path()
                .ok_or_else(|| IpcError::new("no_config_dir", "cannot resolve presets file location"))?;
            load_presets(&path).map_err(|e| IpcError::new("preset_error", format!("{e:?}")))?
        } else {
            Vec::new()
        };

        let passes: Vec<PassSelection> = enabled()
            .map(|dto| {
                let preset = dto.preset_id.as_deref().and_then(|id| presets.iter().find(|p| p.id == id));
                let override_ = SettingsOverride {
                    speed: dto.speed,
                    force: dto.force,
                    repeat_count: dto.repeat_count,
                };
                PassSelection { color: dto.color, settings: resolve_settings(preset, &override_) }
            })
            .collect();

        // The wire carries the revision as a string. One that isn't a u64 was
        // never issued by `doc_revision`, so it cannot be the current plan.
        let Ok(expected) = request.doc_revision.parse::<u64>() else {
            return Err(IpcError::new("stale_plan", "cut request carries an unrecognized plan revision"));
        };
        let opts = PlanOptions { passes, expect_revision: Some(expected), allow_out_of_bounds: false };

        // Planned here, at cut time, against the live document — `expect_revision`
        // is what refuses the cut if that is no longer the document the UI planned.
        let planned = plan_passes(&app.editor.doc)
            .map_err(|e| IpcError::new("plan_error", e.to_string()))?;
        let plan = plan_cut(&planned, &profile, &caps, &opts).map_err(map_cut_error)?;
        Ok(plan.cut_passes())
    }

    /// Submits already-planned passes to the device manager. Blocks until the
    /// worker reaches its first pause point or completion — call this off the
    /// document lock (see `prepare_cut`) and from an async command so it
    /// doesn't freeze the Tauri main loop.
    pub fn execute_cut(&self, passes: Vec<CutPass>) -> Result<u64, IpcError> {
        let aimed = self.connected.lock().unwrap().clone();
        match aimed.as_ref().map(|d| self.route(d)).transpose()? {
            None | Some(Route::Local) => {
                self.manager()?.cut(passes).map_err(|e| IpcError::new("device_error", format!("{e:?}")))
            }
            Some(Route::Host(id)) => {
                // `execute_cut` takes only the Passes, so both the device and the machine it is
                // for come from what the dialog is aimed at — which is also what `route` just
                // resolved, so the two cannot disagree.
                let aimed = aimed.expect("a route implies a device");
                let (device, machine_id) = (aimed.instance_id, aimed.machine_id);
                // A fresh id per attempt: this is a new Job, not a retry of a dropped reply,
                // and reusing one would make the host treat it as already accepted.
                let dispatch_id = cut_host::protocol::DispatchId(format!(
                    "{}-{}",
                    device,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0)
                ));
                self.with_host(&id, |c| c.dispatch(dispatch_id, &device, &machine_id, passes))?;
                // ponytail: a remote dispatch reports job id 0, because `Response::Accepted` carries none —
                // `DeviceManager::cut` does not return one until the Job reaches a pause point. Nothing reads
                // this value for a remote cut today; give it the real id when the desktop shows per-Job history.
                Ok(0)
            }
        }
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

/// Every way `plan_cut` can refuse, as an IPC code the UI can branch on.
/// `stale_plan` is the one the frontend actually keys off (CutDialog.tsx).
/// The message is `cutplan`'s — this used to restate all ten here, and the CLI
/// restated them again, differently.
fn map_cut_error(e: CutError) -> IpcError {
    IpcError::new(e.code(), e.to_string())
}

#[derive(Debug, Serialize)]
pub struct PlanCutResponse {
    pub passes: Vec<PlanCutPassSummary>,
    pub skipped_no_stroke: usize,
    pub doc_revision: String,
    pub travel: Vec<[f64; 4]>,
}

#[derive(Debug, Serialize)]
pub struct PlanCutPassSummary {
    pub color: Option<u32>,
    pub shape_count: usize,
    pub node_ids: Vec<document::NodeId>,
}

/// Summarizes `plan_passes` output for the UI — not the raw `DocumentPasses`
/// (which carries full flattened polylines the cut dialog doesn't need).
pub fn plan_cut_response(doc: &document::Document) -> Result<PlanCutResponse, IpcError> {
    let planned = plan_passes(doc).map_err(|e| IpcError::new("plan_error", e.to_string()))?;
    let refs: Vec<&ColorPass> = planned.passes.iter().collect();
    let travel = cutplan::travel_moves(&refs);
    Ok(PlanCutResponse {
        passes: planned.passes.iter().map(|p| PlanCutPassSummary {
            color: p.color,
            shape_count: p.shapes.len(),
            node_ids: p.shapes.iter().map(|s| s.node_id).collect(),
        }).collect(),
        skipped_no_stroke: planned.skipped_no_stroke,
        doc_revision: planned.doc_revision.to_string(),
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
    use cutplan::DocumentPasses;
    use driver_core::{Driver, Job, MachineCaps, MachineProfile, Phase, Transport, TransportError, TransportKind};

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
        fn driver_for(&self, machine_id: &str) -> Option<Box<dyn Driver + Send>> {
            // Mirrors the real registry: an id nobody claims resolves to nothing,
            // rather than silently handing back some other machine's encoder.
            if machine_id != "cameo5" {
                return None;
            }
            Some(Box::new(TestDriver {
                profile: MachineProfile { id: "cameo5".into(), name: "Test Cameo".into(), width_mm: 500.0, height_mm: 500.0 },
                // A machine that cannot be polled parks the cut at `AwaitingConfirmation`
                // instead of driving it to completion, so a cut submitted here stops at a
                // stable mid-flight phase. `MockTransport` answers no status query, so a
                // pollable machine would instead sit out the manager's 60s completion
                // budget and then fail.
                caps: MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true },
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
            host: None,
        }
    }

    fn test_device_setup() -> DeviceManagerHandle {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        dev.connect(test_instance()).unwrap();
        dev
    }

    fn plan_for(app: &AppState) -> DocumentPasses {
        plan_passes(&app.editor.doc).unwrap()
    }

    fn request_from(plan: DocumentPasses) -> CutRequest {
        CutRequest {
            device_instance_id: test_instance().instance_id,
            doc_revision: plan.doc_revision.to_string(),
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
    fn preflight_failures_map_to_ipc_codes() {
        let app = AppState::new();
        let dev = test_device_setup();
        let revision = cutplan::doc_revision(&app.editor.doc);
        let request = CutRequest { device_instance_id: test_instance().instance_id, doc_revision: revision.to_string(), passes: vec![] };
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

    /// The bridge used to synthesize `Transmitting` from `Progress` because the
    /// worker never re-emitted a state mid-transmit. `CutStatus` carries the
    /// progress in the phase itself, so the synthesis has nothing left to do —
    /// note that nothing here consumes the event channel at all.
    #[test]
    fn status_reports_a_mid_flight_cut_without_the_bridge_synthesizing_it() {
        let mut app = AppState::new();
        let dev = test_device_setup();
        app.add_rect(10.0, 10.0);
        let plan = plan_for(&app);
        dev.cut_from_request(&app, request_from(plan)).expect("cut");
        let s = dev.status();
        // The exact phase, not merely a mid-flight one: a fixture that let the cut
        // run to completion would leave nothing in flight for the status to report,
        // and this test would then pass without ever exercising the question.
        assert_eq!(s.phase, Phase::AwaitingConfirmation, "the cut should be parked mid-flight");
        assert!(s.is_active(), "a parked cut is what makes the close guard block a quit");
    }

    #[test]
    fn status_without_a_manager_reads_disconnected() {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        dev.shutdown();
        assert_eq!(dev.status().phase, Phase::Disconnected);
    }

    #[test]
    fn caps_for_returns_the_drivers_own_answer() {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        let caps = dev.caps_for("cameo5").expect("known machine id");
        assert_eq!(
            caps,
            MachineCaps { supports_speed: true, supports_force: true, needs_operator_pass_confirm: true }
        );
    }

    #[test]
    fn caps_for_unknown_machine_is_an_error_not_a_default() {
        let (dev, _events) = DeviceManagerHandle::new(Arc::new(TestFactory));
        let err = dev.caps_for("nope").expect_err("no driver claims this id");
        assert_eq!(err.code, "unknown_machine");
    }

    /// The UI reads `caps.supportsSpeed`. Drop the serde rename and it reads
    /// `undefined`, `!undefined` is `true`, and every field greys out on every
    /// machine — silent, and wrong in the direction that looks plausible.
    #[test]
    fn machine_caps_serializes_in_the_casing_the_ui_reads() {
        let json = serde_json::to_value(MachineCaps {
            supports_speed: true,
            supports_force: false,
            needs_operator_pass_confirm: true,
        })
        .unwrap();
        assert_eq!(json["supportsSpeed"], serde_json::json!(true));
        assert_eq!(json["supportsForce"], serde_json::json!(false));
        assert_eq!(json["needsOperatorPassConfirm"], serde_json::json!(true));
    }

    use crate::hosts::PairedHost;
    use driver_core::HostId;

    fn a_paired_host(id: &str, addr: &str) -> PairedHost {
        PairedHost {
            id: HostId(id.into()),
            name: "Workshop Pi".into(),
            address: addr.into(),
            fingerprint: "aa:bb:cc".into(),
            token: "s3cret".into(),
        }
    }

    /// A user who never pairs a Pi must see exactly what they see today. This is the test that
    /// says the feature is optional by construction rather than by intention.
    #[test]
    fn with_no_host_paired_the_device_list_is_the_local_one() {
        let dev = test_device_setup();
        let listed = dev.list_devices();

        // Not just "nothing is host-tagged" — an empty list satisfies that vacuously, and the
        // regression this guards against is exactly one that returns nothing.
        assert!(!listed.is_empty(), "the local factory's devices must still be listed");
        assert_eq!(listed.len(), TestFactory.list_devices().len());
        assert!(listed.iter().all(|d| d.host.is_none()), "{listed:?}");
    }

    /// A host that cannot be reached keeps its place in the list rather than vanishing — a
    /// cutter that disappears looks like one that was never paired.
    #[test]
    fn an_unreachable_host_is_still_listed_with_its_reason() {
        let dev = test_device_setup();
        // Nothing is listening on this port, so connecting fails.
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        // `add_host` records the host without dialling it — connections are lazy — so the
        // failure only exists once something asks for its cutters.
        let listed = dev.list_devices();
        assert!(listed.iter().all(|d| d.host.is_none()), "an unreachable host contributes none");

        let views = dev.host_views();
        assert_eq!(views.len(), 1, "the host stays known: {views:?}");
        assert!(views[0].unreachable.is_some(), "and says why it is unreachable");
    }

    /// A cutter with no host is this computer's, and must reach the local DeviceManager — the
    /// path every existing user is on.
    #[test]
    fn a_local_device_still_routes_to_the_local_manager() {
        let dev = test_device_setup();
        assert_eq!(dev.status().phase, driver_core::Phase::Idle);
        assert!(dev.cancel().is_ok(), "a local cancel reaches the local manager");
    }

    /// Naming a host that was forgotten (or never paired) must be refused rather than falling
    /// back to the local cutter — a Job aimed at a Pi must never be cut on the desk.
    #[test]
    fn a_device_naming_an_unknown_host_is_refused_not_run_locally() {
        let dev = test_device_setup();
        let elsewhere = DeviceInfo {
            host: Some(HostId("host-does-not-exist".into())),
            ..test_instance()
        };
        let err = dev.connect(elsewhere).unwrap_err();
        assert_eq!(err.code, "unknown_host", "got {err:?}");
    }

    /// A device aimed at a host that was forgotten (or never paired) must report a status the UI
    /// cannot act on. Asserted on `actions`, not `phase` — this project's rule is that a caller
    /// learns what is legal from `actions` and never re-derives it from the phase.
    #[test]
    fn status_for_a_device_on_an_unknown_host_is_not_a_local_idle() {
        let dev = test_device_setup();
        dev.connected.lock().unwrap().replace(DeviceInfo {
            host: Some(HostId("host-does-not-exist".into())),
            ..test_instance()
        });
        assert!(!dev.status().actions.cut, "an unreachable host must not offer a cut it cannot run");
    }

    #[test]
    fn forgetting_a_host_removes_it_and_its_cutters() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        assert_eq!(dev.host_views().len(), 1, "known as soon as it is added");

        dev.remove_host(&HostId("host-1".into()));
        assert!(dev.host_views().is_empty());
        assert!(dev.list_devices().iter().all(|d| d.host.is_none()));
    }

    /// The token must not leave the Rust side. A view type is the guard, and this is what stops
    /// a later refactor from "simplifying" it back to sending `PairedHost`.
    #[test]
    fn a_host_view_carries_no_token() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        // `unreachable` is `None` here on purpose: nothing has dialled this host yet, and the
        // view reports what is known rather than provoking a connection to find out.
        let views = dev.host_views();
        assert_eq!(views.len(), 1);
        let json = serde_json::to_string(&views[0]).unwrap();
        assert!(!json.contains("s3cret"), "a token reached the view: {json}");
        assert!(json.contains("host-1"), "the id is what the UI addresses: {json}");
    }

    #[test]
    fn pairing_mints_an_id_that_does_not_collide() {
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));
        let next = crate::hosts::next_id(&dev.paired_hosts());
        assert_eq!(next, HostId("host-2".into()));
    }

    // --- pair()/forget(): network refusals, persist-then-mutate, busy-host refusal ---
    //
    // `crates/cut-host/tests/fixtures/mod.rs` has this exact fixture already, but it lives in a
    // separate integration-test crate and cannot be imported from here — so this is the same
    // loopback host, built from the same public `cut_host` API, kept alive by the same
    // `_dir: TempDir` trick (dropping it deletes the cert directory `serve_on` reads its key from).

    struct LoopbackHost {
        addr: String,
        fingerprint: String,
        _dir: tempfile::TempDir,
    }

    const HOST_TOKEN: &str = "test-token";

    fn start_loopback_host() -> LoopbackHost {
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let config = cut_host::config::Config {
            bind: listener.local_addr().unwrap(),
            tokens: [("test-client".to_string(), HOST_TOKEN.to_string())].into_iter().collect(),
            max_frame: cut_host::frame::DEFAULT_MAX_FRAME,
            cert_dir: dir.path().to_path_buf(),
        };
        // Generated before `Host::start`, so the client has something to pin before the
        // server has accepted anything.
        let fingerprint = cut_host::serve::fingerprint_of_cert_dir(&config.cert_dir).unwrap();
        let host = cut_host::host::Host::start(Arc::new(cut_host::host::testing::TwoCutterFactory));

        std::thread::spawn(move || {
            let _ = cut_host::serve::serve_on(listener, host, config);
        });
        LoopbackHost { addr, fingerprint, _dir: dir }
    }

    /// A regular file where a directory needs to be, so `create_dir_all` — and therefore every
    /// `hosts::save` — fails without touching real filesystem permissions.
    fn unwritable_hosts_path(dir: &std::path::Path) -> std::path::PathBuf {
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"").unwrap();
        blocker.join("hosts.json")
    }

    #[test]
    fn pairing_with_the_wrong_token_is_refused_and_saves_nothing() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let err = dev.pair("Pi".into(), host.addr.clone(), "wrong-token".into(), host.fingerprint.clone(), &hosts_path);
        assert!(err.is_err());
        assert!(!hosts_path.exists(), "a pairing that never proved itself must not be written");
    }

    #[test]
    fn pairing_with_the_wrong_fingerprint_is_refused_and_saves_nothing() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let err = dev.pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), "wrong:fingerprint".into(), &hosts_path);
        assert!(err.is_err());
        assert!(!hosts_path.exists());
    }

    #[test]
    fn pairing_an_unreachable_address_is_refused_and_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        // The conventional black-holed address: routable, never answering.
        let err = dev.pair("Pi".into(), "10.255.255.1:7878".into(), "token".into(), "aa:bb:cc".into(), &hosts_path);
        assert!(err.is_err());
        assert!(!hosts_path.exists());
    }

    #[test]
    fn a_pair_that_fails_to_save_adds_nothing_in_memory() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = unwritable_hosts_path(dir.path());
        let dev = test_device_setup();

        let err = dev.pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path);
        assert!(err.is_err(), "the save must fail against this path");
        assert!(dev.paired_hosts().is_empty(), "a failed save must not leave a phantom host in memory");
    }

    #[test]
    fn a_forget_that_fails_to_save_removes_nothing_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = unwritable_hosts_path(dir.path());
        let dev = test_device_setup();
        dev.add_host(a_paired_host("host-1", "127.0.0.1:1"));

        let err = dev.forget(&HostId("host-1".into()), &hosts_path);
        assert!(err.is_err(), "the save must fail against this path");
        assert_eq!(dev.paired_hosts().len(), 1, "a failed save must not remove a host still on disk");
    }

    #[test]
    fn a_host_with_an_active_cut_refuses_to_be_forgotten() {
        use std::time::{Duration, Instant};

        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("this host answers and the fingerprint matches");

        // `dispatch` returns as soon as the daemon accepts the id; the cut itself runs on a
        // thread there (`Host::dispatch`). Poll until the cutter reports busy rather than
        // asserting on a race.
        let client = cut_host::client::HostClient::connect(&host.addr, HOST_TOKEN, &host.fingerprint).unwrap();
        client
            .dispatch(
                cut_host::protocol::DispatchId("d-1".into()),
                cut_host::host::testing::CAMEO,
                "cameo5",
                vec![CutPass {
                    job: driver_core::Job {
                        polylines: vec![vec![
                            geometry::Point { x: 0.0, y: 0.0 },
                            geometry::Point { x: 10.0, y: 0.0 },
                            geometry::Point { x: 10.0, y: 10.0 },
                            geometry::Point { x: 0.0, y: 0.0 },
                        ]],
                        settings: driver_core::Settings::default(),
                    },
                }],
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let busy = client
                .snapshots()
                .unwrap()
                .iter()
                .any(|s| s.info.instance_id == cut_host::host::testing::CAMEO && s.status.is_active());
            if busy {
                break;
            }
            assert!(Instant::now() < deadline, "the cutter never went active");
            std::thread::sleep(Duration::from_millis(20));
        }

        let err = dev.forget(&id, &hosts_path).expect_err("a moving blade must not become unstoppable");
        assert_eq!(err.code, "host_busy");
        assert_eq!(dev.paired_hosts().len(), 1, "refused, so still paired");
    }

    #[test]
    fn an_idle_host_is_forgotten() {
        let host = start_loopback_host();
        let dir = tempfile::tempdir().unwrap();
        let hosts_path = dir.path().join("hosts.json");
        let dev = test_device_setup();

        let id = dev
            .pair("Pi".into(), host.addr.clone(), HOST_TOKEN.into(), host.fingerprint.clone(), &hosts_path)
            .expect("this host answers and the fingerprint matches");

        dev.forget(&id, &hosts_path).expect("nothing is running on it");
        assert!(dev.paired_hosts().is_empty());
    }
}
