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

/// Stops what this process is driving, shuts the device manager down and exits. Used by the UI
/// after the operator confirms they want to quit with a cut outstanding.
///
/// A Job on a Cut Host is left running: the host owns it and keeps cutting whether this desktop is
/// alive or not, while the local cutter's transport dies with this process, so what quitting can
/// honestly stop is only ever the local one (#158, and
/// `docs/adr/0002-the-close-guard-answers-for-every-cut-this-desktop-started.md`). The dialog the
/// operator answered says so.
///
/// async because `shutdown` joins the local worker, and this runs on the main thread — the escape
/// hatch from the close guard hanging harder than the guard it escapes. It no longer touches the
/// network at all, which is what ends #116's cause rather than merely bounding it: the remote
/// cancel that could block the exit is gone.
#[tauri::command(async)]
fn force_quit(app: tauri::AppHandle, dev: tauri::State<DeviceManagerHandle>) {
    // The operator has answered the prompt, so nothing new may start: a press already queued for a
    // host's connection must not send a Job into a process that is exiting.
    dev.commit_close_after_confirmation();
    dev.stop_local_motion();
    dev.shutdown();
    app.exit(0);
}

/// The webview's acknowledgement that its quit-warning listener is installed.
///
/// Tauri's `emit` returns success with zero listeners, so delivery cannot be inferred from the
/// result. The webview sets this only after `listen` resolves and clears it before teardown; a
/// close arriving outside that lifetime proceeds instead of becoming a refusal with no escape.
#[tauri::command]
fn set_close_guard_ready(dev: tauri::State<DeviceManagerHandle>, ready: bool) {
    dev.set_close_guard_ready(ready);
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
            ipc::set_cut_line_type,
            ipc::set_material_preset,
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
            ipc::travel_for_order,
            ipc::cut,
            ipc::cancel_cut,
            ipc::resume_cut,
            ipc::confirm_pass_done,
            ipc::list_presets,
            ipc::machine_caps,
            ipc::settings_ranges,
            ipc::save_preset,
            ipc::delete_preset,
            ipc::list_hosts,
            ipc::probe_host,
            ipc::existing_pairing,
            ipc::test_host,
            ipc::pair_host,
            ipc::forget_host,
            ipc::trace_image,
            ipc::trace_controls,
            ipc::list_fonts,
            ipc::load_image_preview,
            ipc::pick_image,
            set_close_guard_ready,
            force_quit,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let dev = window.state::<DeviceManagerHandle>();
                // Never blocking, local or remote. This callback is synchronous and runs on the
                // main thread, and `status()` aimed at a host dials: its 2s budget starts only
                // once that host's connection lock is in hand, and `list_devices` holds the same
                // lock across a 30s call — so a close arriving during a device-list poll froze
                // the window for the listing plus the poll plus an unbounded resolve (#115).
                // What this asks is whether to warn, which a status alone cannot answer: a
                // dispatch accepted a second ago has not been polled yet, so the newest status
                // anyone holds is the `Idle` from before it. `a_cut_may_be_running` counts every
                // dispatch this desktop sent and has not seen finish, on any cutter and whatever
                // is aimed at now (#158); a warning about a Job that has since ended costs a
                // dialog the operator dismisses.
                // `commit_close` answers and commits in one step, so a press queued behind a poll
                // for a host's connection cannot cross into its dispatch after the window has been
                // let go: the Job it would start outlives the process that started it.
                if dev.commit_close() {
                    return;
                }
                if dev.close_guard_ready() {
                    // Readiness is the delivery fact: `emit` only proves serialization and returns
                    // success even with nobody listening. Once the webview has acknowledged its
                    // listener, a successful emit and the refusal are one act.
                    match window.emit("cut-in-progress", ()) {
                        Ok(()) => api.prevent_close(),
                        Err(e) => eprintln!(
                            "cuthulhu: a cut may be running and the warning could not be shown: {e}"
                        ),
                    }
                } else {
                    // Fail open by design — a refused close with no listener has no escape — but
                    // not silently: this is the only evidence a startup/listener failure disabled
                    // the guard when an operator later reports a warning was missing.
                    eprintln!(
                        "cuthulhu: a cut may be running but the warning listener is not ready; closing"
                    );
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
