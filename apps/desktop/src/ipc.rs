// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::PathBuf;
use std::sync::Mutex;
use document::{Delta, MachineProfile, NodeId, ShapeKind};
use driver_core::DeviceInfo;
use driver_core::manager::DeviceState;
use geometry::{Affine, BoolOp};
use crate::device::{plan_cut_response, CutRequest, DeviceManagerHandle, IpcError, PlanCutResponse};
use crate::state::AppState;
use cutplan::presets::MaterialPreset;

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
pub fn undo(state: tauri::State<AppStateHandle>) -> Result<Option<Delta>, String> {
    Ok(state.lock().unwrap().undo())
}

#[tauri::command]
pub fn redo(state: tauri::State<AppStateHandle>) -> Result<Option<Delta>, String> {
    Ok(state.lock().unwrap().redo())
}

#[tauri::command]
pub fn import_svg(state: tauri::State<AppStateHandle>, bytes: Vec<u8>, parent: NodeId) -> Result<(Delta, Vec<String>), String> {
    state.lock().unwrap().import_svg(bytes, parent).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn save_project(state: tauri::State<AppStateHandle>, path: PathBuf) -> Result<(), String> {
    state.lock().unwrap().save_project(&path).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn load_project(state: tauri::State<AppStateHandle>, path: PathBuf) -> Result<String, String> {
    state.lock().unwrap().load_project(&path).map_err(|e| format!("{e:?}"))
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

#[tauri::command]
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

// Non-blocking, event-driven cache — safe to call even while a cut is
// mid-transmit (never touches the worker thread; see DeviceManagerHandle::cached_state).
#[tauri::command]
pub fn get_device_state(dev: tauri::State<DeviceManagerHandle>) -> Result<DeviceState, IpcError> {
    Ok(dev.cached_state())
}

#[tauri::command]
pub fn plan_cut(state: tauri::State<AppStateHandle>) -> Result<PlanCutResponse, IpcError> {
    plan_cut_response(&state.lock().unwrap().editor.doc)
}

// async: prepare_cut briefly locks the document (plan + preflight), then the
// lock is dropped before execute_cut's blocking call into the device worker
// so `cut` never holds the doc mutex while blocked, and running off the main
// loop keeps the UI (and cancel_cut) responsive while it blocks.
#[tauri::command(async)]
pub fn cut(state: tauri::State<AppStateHandle>, dev: tauri::State<DeviceManagerHandle>, request: CutRequest) -> Result<u64, IpcError> {
    let passes = {
        let app = state.lock().unwrap();
        dev.prepare_cut(&app, request)?
    };
    dev.execute_cut(passes)
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
    crate::device::list_presets(&machine_id)
}

#[tauri::command]
pub fn save_preset(p: MaterialPreset) -> Result<(), IpcError> {
    crate::device::save_preset(p)
}

#[tauri::command]
pub fn delete_preset(id: String) -> Result<(), IpcError> {
    crate::device::delete_preset(&id)
}
