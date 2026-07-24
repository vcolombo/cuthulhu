// SPDX-License-Identifier: GPL-3.0-or-later
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use std::time::{Duration, Instant};

use driver_core::manager::DeviceEventKind;
use tauri::{Emitter, Manager};

use desktop::device::{is_active, DesktopBackendFactory, DeviceManagerHandle};
use desktop::ipc;
use desktop::state::AppState;

/// Cancels + shuts down the device manager and exits the process. Used by the
/// UI after the user confirms they want to quit with a cut in progress.
#[tauri::command]
fn force_quit(app: tauri::AppHandle, dev: tauri::State<DeviceManagerHandle>) {
    dev.cancel().ok();
    dev.shutdown();
    app.exit(0);
}

fn main() {
    let (dev_handle, events) = DeviceManagerHandle::new(std::sync::Arc::new(DesktopBackendFactory));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(AppState::new()))
        .manage(dev_handle)
        .invoke_handler(tauri::generate_handler![
            ipc::new_doc,
            ipc::snapshot,
            ipc::commit_transform,
            ipc::add_primitive,
            ipc::boolean_op,
            ipc::add_text,
            ipc::delete,
            ipc::reorder,
            ipc::undo,
            ipc::redo,
            ipc::import_svg,
            ipc::save_project,
            ipc::load_project,
            ipc::set_machine,
            ipc::list_machines,
            ipc::list_devices,
            ipc::connect_device,
            ipc::disconnect_device,
            ipc::get_device_state,
            ipc::plan_cut,
            ipc::cut,
            ipc::cancel_cut,
            ipc::resume_cut,
            ipc::confirm_pass_done,
            ipc::list_presets,
            ipc::save_preset,
            ipc::delete_preset,
            force_quit,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let dev = window.state::<DeviceManagerHandle>();
                // Non-blocking cached read: the worker thread may be busy
                // mid-transmit, and this handler must never block on it.
                if is_active(&dev.cached_state()) {
                    api.prevent_close();
                    window.emit("cut-in-progress", ()).ok();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Event bridge: sole consumer of the device-event channel. Coalesces
    // `Progress` to <=10Hz (drop intermediate ticks); every other event kind
    // is forwarded immediately. Also the sole writer of DeviceManagerHandle's
    // cached state (record_state), so get_device_state and the close handler
    // above never block on the worker thread. Dropped webview listeners are a
    // normal `emit` no-op, not an error.
    let bridge_handle = app.handle().clone();
    std::thread::spawn(move || {
        let mut last_progress: Option<Instant> = None;
        for event in events {
            bridge_handle.state::<DeviceManagerHandle>().record_state(&event);
            if matches!(event.kind, DeviceEventKind::Progress { .. }) {
                let now = Instant::now();
                if last_progress.is_some_and(|last| now.duration_since(last) < Duration::from_millis(100)) {
                    continue;
                }
                last_progress = Some(now);
            }
            let _ = bridge_handle.emit("device-event", &event);
        }
    });

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            app_handle.state::<DeviceManagerHandle>().shutdown();
        }
    });
}
