// SPDX-License-Identifier: GPL-3.0-or-later
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use std::time::{Duration, Instant};

use driver_core::manager::DeviceEventKind;
use tauri::{Emitter, Manager};

use desktop::device::DeviceManagerHandle;
use driver_registry::HardwareBackendFactory;
use desktop::hosts;
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
    let (dev_handle, events) = DeviceManagerHandle::new(std::sync::Arc::new(HardwareBackendFactory));

    // A host that fails to load is not a reason to refuse to start — the desktop still cuts on
    // local hardware, and the operator can re-pair. Say so once rather than failing silently.
    let paired = hosts::load_or_warn(hosts::default_hosts_path().as_deref(), |e| {
        eprintln!("cuthulhu: paired hosts could not be loaded: {e}")
    });
    for host in paired {
        dev_handle.add_host(host);
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(AppState::new()))
        .manage(dev_handle)
        .manage(ipc::AuthorizedImages::default())
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
            ipc::reconnect_device,
            ipc::get_device_state,
            ipc::get_connected_device,
            ipc::plan_cut,
            ipc::cut,
            ipc::cancel_cut,
            ipc::resume_cut,
            ipc::confirm_pass_done,
            ipc::list_presets,
            ipc::machine_caps,
            ipc::save_preset,
            ipc::delete_preset,
            ipc::list_hosts,
            ipc::probe_host,
            ipc::test_host,
            ipc::pair_host,
            ipc::forget_host,
            ipc::trace_image,
            ipc::trace_controls,
            ipc::load_image_preview,
            ipc::pick_image,
            force_quit,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let dev = window.state::<DeviceManagerHandle>();
                // Non-blocking for a local device: the worker publishes status rather than being
                // asked for it. Aimed at a host, this is a synchronous network round trip on the
                // UI thread — bounded (roughly 2x STATUS_POLL_TIMEOUT, see DeviceManagerHandle::
                // status) but not instant, and DNS ahead of it is unbounded with std. A closing
                // window can briefly stall on a wedged Pi; it will not hang forever.
                if dev.status().is_active() {
                    api.prevent_close();
                    window.emit("cut-in-progress", ()).ok();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Event bridge: sole consumer of the device-event channel. Coalesces
    // `Progress` to <=10Hz (drop intermediate ticks) because each forward costs a
    // webview emit, not because the cut cares; every other event kind is forwarded
    // immediately. A dropped tick loses nothing — each event carries the status that
    // held when it was sent, and `get_device_state` reads the same published value.
    // Dropped webview listeners are a normal `emit` no-op, not an error.
    let bridge_handle = app.handle().clone();
    std::thread::spawn(move || {
        let mut last_progress: Option<Instant> = None;
        for event in events {
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
