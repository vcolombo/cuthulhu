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

// Non-blocking cache read, same shape as get_device_state — lets the UI recover which
// device is connected after reopening the cut dialog, without a redundant reconnect.
#[tauri::command]
pub fn get_connected_device(dev: tauri::State<DeviceManagerHandle>) -> Result<Option<DeviceInfo>, IpcError> {
    Ok(dev.connected.lock().unwrap().clone())
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

/// Cap on the source file both trace commands will pull into memory. The decoder's own ceiling
/// only applies once the bytes are already resident, so without this a huge file exhausts memory
/// before it can be rejected for not being a usable image.
const MAX_INPUT_FILE_BYTES: u64 = 256 * 1024 * 1024;

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

/// Read a whole stream, refusing input longer than `cap` bytes.
///
/// `cap` is a parameter rather than the constant so the bound can be exercised with a handful of
/// bytes instead of a quarter gigabyte.
fn read_capped<R: std::io::Read>(reader: R, cap: u64) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    // One byte past the ceiling, so landing exactly on it is distinguishable from exceeding it,
    // and so an oversized input costs one extra byte rather than its whole length.
    reader.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Ok(None);
    }
    Ok(Some(bytes))
}

/// Read an authorized image, refusing anything past the ceiling.
///
/// One open handle, bounded by `read_capped`, rather than `metadata` followed by a separate
/// `std::fs::read`. Two pathname resolutions describe two moments: the size that passed the check
/// belonged to whatever the path pointed at then, and a file that grew in between was read in full
/// anyway. Bounding the read itself holds whatever the size was, or became.
fn read_image_file(path: &PathBuf) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    match read_capped(file, MAX_INPUT_FILE_BYTES)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
    {
        Some(bytes) => Ok(bytes),
        None => Err(format!(
            "file is too large to open: over {} MiB",
            MAX_INPUT_FILE_BYTES / (1024 * 1024)
        )),
    }
}

#[tauri::command(async)]
pub fn trace_image(
    auth: tauri::State<AuthorizedImages>,
    path: PathBuf,
    opts: trace::TraceOptions,
) -> Result<trace::TraceResult, String> {
    let real = authorized_path(&auth, &path)?;
    let bytes = read_image_file(&real)?;
    trace::trace(&bytes, &opts).map_err(|e| e.to_string())
}

/// Returns the source thumbnail as a re-encoded PNG data URL — never the file's original bytes,
/// so a path that is not a decodable image yields an error rather than its contents.
#[tauri::command(async)]
pub fn load_image_preview(
    auth: tauri::State<AuthorizedImages>,
    path: PathBuf,
) -> Result<String, String> {
    let real = authorized_path(&auth, &path)?;
    let bytes = read_image_file(&real)?;
    let png = trace::preview_png(&bytes).map_err(|e| e.to_string())?;
    use base64::Engine as _;
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ceiling has to come from the read itself. A separate size check describes whatever the
    /// pathname pointed at when it ran, so a file that grows between the check and the read is
    /// read in full despite having just passed the limit — the cap is advisory rather than a
    /// bound. Deliberately exercised through a plain reader, with no file and no metadata call,
    /// because that is the property under test.
    #[test]
    fn read_capped_refuses_a_stream_longer_than_the_cap() {
        use std::io::Read as _;
        let over = std::io::repeat(b'x').take(9);
        assert!(read_capped(over, 8).unwrap().is_none(), "9 bytes must be refused against a cap of 8");
    }

    /// Covers the glue the helper tests cannot: opening the path, threading the real ceiling
    /// through, and handing back the bytes. Both trace commands read through here, so a mistake
    /// in this wiring breaks every trace.
    #[test]
    fn read_image_file_returns_the_contents_of_a_small_file() {
        let path = std::env::temp_dir().join("cuthulhu-read-image-file-test.bin");
        std::fs::write(&path, b"not an image, but bytes are bytes").unwrap();
        let got = read_image_file(&path).expect("a small file must be readable");
        std::fs::remove_file(&path).ok();
        assert_eq!(got, b"not an image, but bytes are bytes");
    }

    #[test]
    fn read_image_file_reports_a_missing_path_with_its_name() {
        let path = std::env::temp_dir().join("cuthulhu-definitely-absent.bin");
        std::fs::remove_file(&path).ok();
        let err = read_image_file(&path).expect_err("a missing file must error");
        assert!(err.contains("cuthulhu-definitely-absent.bin"), "error should name the file: {err}");
    }

    #[test]
    fn read_capped_accepts_a_stream_exactly_at_the_cap() {
        use std::io::Read as _;
        let exact = std::io::repeat(b'x').take(8);
        assert_eq!(read_capped(exact, 8).unwrap().map(|b| b.len()), Some(8));
    }
}
