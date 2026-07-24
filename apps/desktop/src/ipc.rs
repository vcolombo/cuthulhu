// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::PathBuf;
use std::sync::Mutex;
use document::{Delta, MachineProfile, NodeId, ShapeKind};
use geometry::{Affine, BoolOp};
use crate::state::AppState;

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
