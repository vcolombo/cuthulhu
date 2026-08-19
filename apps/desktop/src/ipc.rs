// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::PathBuf;
use std::sync::Mutex;
use document::{CutLineType, Delta, MachineProfile, NodeId, PresetAssignment, ShapeKind};
use driver_core::{CutStatus, DeviceInfo, HostId, MachineCaps};
use geometry::{Affine, BoolOp};
use crate::device::{plan_cut_response, CutRequest, CutStarted, DeviceManagerHandle, ExistingPairing, IpcError, PairedHostView, PlanCutResponse, TravelPassDto};
use crate::state::AppState;
use cutplan::presets::MaterialPreset;
use cutplan::Grouping;

pub type AppStateHandle = Mutex<AppState>;

#[tauri::command]
pub fn new_doc(state: tauri::State<AppStateHandle>) -> Result<String, String> {
    Ok(state.lock().unwrap().new_doc())
}

#[tauri::command]
pub fn snapshot(state: tauri::State<AppStateHandle>) -> Result<String, String> {
    Ok(state.lock().unwrap().snapshot())
}

#[tauri::command]
pub fn commit_transform(state: tauri::State<AppStateHandle>, ids: Vec<NodeId>, m: Affine) -> Result<Delta, String> {
    state.lock().unwrap().commit_transform(ids, m).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn add_primitive(state: tauri::State<AppStateHandle>, parent: NodeId, kind: ShapeKind) -> Result<Delta, String> {
    state.lock().unwrap().add_primitive(parent, kind).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn boolean_op(state: tauri::State<AppStateHandle>, ids: Vec<NodeId>, op: BoolOp) -> Result<Delta, String> {
    state.lock().unwrap().boolean_op(ids, op).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn add_text(state: tauri::State<AppStateHandle>, parent: NodeId, family: String, size_mm: f64, text: String) -> Result<Delta, String> {
    state.lock().unwrap().add_text(parent, family, size_mm, text).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn delete(state: tauri::State<AppStateHandle>, ids: Vec<NodeId>) -> Result<Delta, String> {
    state.lock().unwrap().delete(ids).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn reorder(state: tauri::State<AppStateHandle>, id: NodeId, new_index: usize) -> Result<Delta, String> {
    state.lock().unwrap().reorder(id, new_index).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn set_cut_line_type(state: tauri::State<AppStateHandle>, ids: Vec<NodeId>, value: CutLineType)
    -> Result<Delta, String> {
    state.lock().unwrap().set_cut_line_type(ids, value).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn set_material_preset(state: tauri::State<AppStateHandle>, ids: Vec<NodeId>, value: PresetAssignment)
    -> Result<Delta, String> {
    state.lock().unwrap().set_material_preset(ids, value).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn undo(state: tauri::State<AppStateHandle>) -> Result<Option<Delta>, String> {
    Ok(state.lock().unwrap().undo())
}

#[tauri::command]
pub fn redo(state: tauri::State<AppStateHandle>) -> Result<Option<Delta>, String> {
    Ok(state.lock().unwrap().redo())
}

#[tauri::command]
pub fn import_svg(state: tauri::State<AppStateHandle>, bytes: Vec<u8>, parent: NodeId) -> Result<(Delta, Vec<String>), String> {
    state.lock().unwrap().import_svg(bytes, parent).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_project(state: tauri::State<AppStateHandle>, path: PathBuf) -> Result<(), String> {
    state.lock().unwrap().save_project(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_project(state: tauri::State<AppStateHandle>, path: PathBuf) -> Result<String, String> {
    state.lock().unwrap().load_project(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_machine(state: tauri::State<AppStateHandle>, machine_id: String) -> Result<(), String> {
    state.lock().unwrap().set_machine(&machine_id).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn list_machines(state: tauri::State<AppStateHandle>) -> Result<Vec<MachineProfile>, String> {
    Ok(state.lock().unwrap().list_machines())
}

// --- device / cut / preset commands: operate over DeviceManagerHandle, never AppStateHandle's mutex ---

// async: dials every paired host to refresh its device list (30s budget per host, by design —
// see DeviceManagerHandle::list_devices) — on the main thread one wedged Pi freezes the window.
#[tauri::command(async)]
pub fn list_devices(dev: tauri::State<DeviceManagerHandle>) -> Result<Vec<DeviceInfo>, IpcError> {
    Ok(dev.list_devices())
}

// async: worker may be busy mid-transmit, and USB open has real latency.
#[tauri::command(async)]
pub fn connect_device(dev: tauri::State<DeviceManagerHandle>, info: DeviceInfo) -> Result<(), IpcError> {
    dev.connect(info)
}

#[tauri::command(async)]
pub fn disconnect_device(dev: tauri::State<DeviceManagerHandle>) -> Result<(), IpcError> {
    dev.disconnect()
}

// async for the same reasons as connect_device, plus a remote arm that talks to the Pi.
#[tauri::command(async)]
pub fn reconnect_device(dev: tauri::State<DeviceManagerHandle>) -> Result<(), IpcError> {
    dev.reconnect()
}

// async: non-blocking for a local device — the status is published rather than asked of the
// worker (see DeviceManagerHandle::status) — but for a remote one this polls the host over the
// network, bounded by roughly 2x STATUS_POLL_TIMEOUT (reconnect leg, then body-read leg) with
// the DNS/mDNS resolve bounded inside the first leg's deadline, and it is called after every
// connect/cancel/resume/confirm, so it must not run on the main thread.
#[tauri::command(async)]
pub fn get_device_state(dev: tauri::State<DeviceManagerHandle>) -> Result<CutStatus, IpcError> {
    Ok(dev.status())
}

// Non-blocking cache read, same shape as get_device_state — lets the UI recover which
// device is connected after reopening the cut dialog, without a redundant reconnect.
#[tauri::command]
pub fn get_connected_device(dev: tauri::State<DeviceManagerHandle>) -> Result<Option<DeviceInfo>, IpcError> {
    Ok(dev.connected.lock().unwrap().clone())
}

#[tauri::command]
pub fn plan_cut(state: tauri::State<AppStateHandle>, grouping: Grouping) -> Result<PlanCutResponse, IpcError> {
    plan_cut_response(&state.lock().unwrap().editor.doc, grouping)
}

#[tauri::command]
pub fn travel_for_order(
    state: tauri::State<AppStateHandle>,
    doc_revision: String,
    grouping: Grouping,
    passes: Vec<TravelPassDto>,
) -> Result<Vec<[f64; 4]>, IpcError> {
    // Fully qualified because the command and the function it forwards to share a name.
    crate::device::travel_for_order(&state.lock().unwrap().editor.doc, &doc_revision, grouping, &passes)
}

// async: prepare_cut briefly locks the document (plan + preflight), then the
// lock is dropped before execute_cut's blocking call into the device worker
// so `cut` never holds the doc mutex while blocked, and running off the main
// loop keeps the UI (and cancel_cut) responsive while it blocks.
#[tauri::command(async)]
pub fn cut(state: tauri::State<AppStateHandle>, dev: tauri::State<DeviceManagerHandle>, request: CutRequest) -> Result<CutStarted, IpcError> {
    let (planned_for, passes) = {
        let app = state.lock().unwrap();
        dev.prepare_cut(&app, request)?
    };
    dev.execute_cut(planned_for, passes)
}

#[tauri::command(async)]
pub fn cancel_cut(dev: tauri::State<DeviceManagerHandle>) -> Result<(), IpcError> {
    dev.cancel()
}

// async: blocks like `cut` while the worker drives the next pass.
#[tauri::command(async)]
pub fn resume_cut(dev: tauri::State<DeviceManagerHandle>) -> Result<(), IpcError> {
    dev.resume()
}

#[tauri::command(async)]
pub fn confirm_pass_done(dev: tauri::State<DeviceManagerHandle>) -> Result<(), IpcError> {
    dev.confirm_pass_done()
}

#[tauri::command]
pub fn list_presets(machine_id: String) -> Result<Vec<MaterialPreset>, IpcError> {
    crate::device::list_presets(&presets_path()?, &machine_id)
}

#[tauri::command]
pub fn machine_caps(dev: tauri::State<DeviceManagerHandle>, machine_id: String) -> Result<MachineCaps, IpcError> {
    dev.caps_for(&machine_id)
}

#[tauri::command]
pub fn save_preset(p: MaterialPreset) -> Result<(), IpcError> {
    crate::device::save_preset(&presets_path()?, p)
}

/// Keyed on the machine as well as the id: a preset id is the operator's own string, so the same
/// id can name a Cameo's material and a Puma's, and a delete that named only the id removed both
/// (#153).
#[tauri::command]
pub fn delete_preset(machine_id: String, id: String) -> Result<(), IpcError> {
    crate::device::delete_preset(&presets_path()?, &machine_id, &id)
}

fn presets_path() -> Result<PathBuf, IpcError> {
    cutplan::presets::default_presets_path()
        .ok_or_else(|| IpcError::new("no_config_dir", "cannot resolve presets file location"))
}

// async: reads each paired host's connection in the same order `list_devices` dials them, so
// against a sweep of unreachable hosts it trails one host behind and can approach the sweep's
// total — on the main thread that is a frozen window, not just a slow one.
#[tauri::command(async)]
pub fn list_hosts(dev: tauri::State<DeviceManagerHandle>) -> Result<Vec<PairedHostView>, IpcError> {
    Ok(dev.host_views())
}

/// Learn the fingerprint a host presents, so the pairing dialog can show it for confirmation
/// before anything is pinned. The first step of a first pairing, ahead of `test_host`: until the
/// operator has confirmed it there is no fingerprint to hand `test_host`.
///
/// async: the probe dials the host the same way `test_host` does (see there for why).
#[tauri::command(async)]
pub fn probe_host(address: String) -> Result<String, IpcError> {
    cut_host::client::probe_fingerprint(&address, cut_host::client::CONNECT_TIMEOUT)
        .map_err(|e| crate::device::host_error(&e))
}

/// Whether this address is already paired, and whether the certificate just probed is the one
/// pinned for it. Asked between the probe and the confirm, so a re-pairing after a Pi's
/// certificate changed is something the operator is told about rather than something they
/// discover as a second, permanently broken row (#107).
#[tauri::command]
pub fn existing_pairing(
    dev: tauri::State<DeviceManagerHandle>,
    address: String,
    fingerprint: String,
) -> Result<Option<ExistingPairing>, IpcError> {
    Ok(dev.existing_pairing(&address, &fingerprint))
}

/// Prove a host without saving it. The pairing dialog calls this before `pair_host`, so an
/// entry that has never worked is never written.
///
/// async: `pair_check` dials the host over the network (5s connect timeout with the DNS/mDNS
/// resolve bounded inside it, 30s body timeout) — on the main thread a mistyped address would
/// freeze the window for the length of that wait.
#[tauri::command(async)]
pub fn test_host(address: String, token: String, fingerprint: String) -> Result<Vec<DeviceInfo>, IpcError> {
    cut_host::client::HostClient::pair_check(&address, &token, &fingerprint)
        .map_err(|e| crate::device::host_error(&e))
}

// async: pairing dials the host the same way `test_host` does (see there for why).
#[tauri::command(async)]
pub fn pair_host(
    dev: tauri::State<DeviceManagerHandle>,
    name: String,
    address: String,
    token: String,
    fingerprint: String,
) -> Result<PairedHostView, IpcError> {
    let path = hosts_path()?;
    let id = dev.pair(name.clone(), address.clone(), token, fingerprint, &path)?;
    Ok(PairedHostView { id, name, address, unreachable: None })
}

// async: the idle check dials the host the same way `test_host` does (see there for why).
//
// `force` is the operator accepting that a host which cannot be asked may still be cutting. The
// UI only offers it after an unforced attempt has already been refused — see `DeviceManagerHandle::forget`.
#[tauri::command(async)]
pub fn forget_host(dev: tauri::State<DeviceManagerHandle>, id: HostId, force: bool) -> Result<(), IpcError> {
    dev.forget(&id, &hosts_path()?, force)
}

fn hosts_path() -> Result<PathBuf, IpcError> {
    crate::hosts::default_hosts_path()
        .ok_or_else(|| IpcError::new("no_config_dir", "this system has no configuration directory"))
}

/// Paths the user has actually chosen in the native picker this session.
///
/// The trace commands take a path from the webview, so without this any code running there could
/// name an arbitrary file and read image content back out. The set is only ever added to by
/// `pick_image`, which is the picker itself — the renderer cannot authorize a path, only ask for
/// one to be authorized by the user choosing it.
#[derive(Default)]
pub struct AuthorizedImages(pub Mutex<std::collections::HashSet<PathBuf>>);

/// Canonicalized so an authorized file cannot be re-reached under a different spelling
/// (`..` segments, a symlink, a relative path) and miss the membership check.
fn canonical(path: &PathBuf) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Returns the *resolved* path to read, so the caller opens exactly what was authorized.
///
/// Returning `()` and letting the caller re-open its own argument would leave a window between
/// the check and the read: the caller's path is resolved twice, and a symlink that pointed at an
/// authorized file for the check can be retargeted before the open. Reading the already-resolved
/// path removes that step entirely.
fn authorized_path(auth: &AuthorizedImages, path: &PathBuf) -> Result<PathBuf, String> {
    let real = canonical(path)?;
    if auth.0.lock().unwrap().contains(&real) {
        Ok(real)
    } else {
        Err("that image was not selected in this session".into())
    }
}

/// Open the native image picker and record what the user chose. This lives in Rust rather than
/// in the webview precisely so that selection, not the caller's say-so, is what grants access.
#[tauri::command(async)]
pub fn pick_image(
    app: tauri::AppHandle,
    auth: tauri::State<AuthorizedImages>,
) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp"])
        .blocking_pick_file();
    let Some(picked) = picked else { return Ok(None) };
    let path = picked
        .into_path()
        .map_err(|e| format!("could not resolve the selected file: {e}"))?;
    let real = canonical(&path)?;
    auth.0.lock().unwrap().insert(real.clone());
    Ok(Some(real))
}

/// The tracer's own description of what it accepts, so the dialog renders its controls from the
/// module that enforces them rather than from a table typed to agree.
#[tauri::command]
pub fn trace_controls() -> Result<trace::TraceControlSpecs, IpcError> {
    Ok(trace::control_specs())
}

// async: load_system_fonts walks the system font directories on disk — keep the scan off the
// main thread, same reasoning as the trace commands. An empty list is honest data (a box with
// no fonts), not an error; the dialog renders it as a state.
#[tauri::command(async)]
pub fn list_fonts() -> Result<Vec<String>, IpcError> {
    Ok(geometry::list_font_families())
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

/// Returns the source thumbnail as a re-encoded PNG data URL — never the file's original bytes,
/// so a path that is not a decodable image yields an error rather than its contents.
#[tauri::command(async)]
pub fn load_image_preview(
    auth: tauri::State<AuthorizedImages>,
    path: PathBuf,
) -> Result<String, String> {
    let real = authorized_path(&auth, &path)?;
    let bytes = trace::read_image(&real).map_err(|e| e.to_string())?;
    let png = trace::preview_png(&bytes).map_err(|e| e.to_string())?;
    use base64::Engine as _;
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}
