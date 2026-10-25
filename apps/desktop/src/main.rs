// SPDX-License-Identifier: GPL-3.0-or-later
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use std::time::{Duration, Instant};

use driver_core::manager::DeviceEventKind;
use tauri::{Emitter, Manager};

use desktop::device::{CloseVerdict, DeviceManagerHandle};
use desktop::hosts;
use desktop::ipc;
use desktop::preferences::ResolvedTheme;
use desktop::state::AppState;
use driver_registry::HardwareBackendFactory;

fn resolved_theme(theme: tauri::Theme) -> ResolvedTheme {
    match theme {
        tauri::Theme::Dark => ResolvedTheme::Dark,
        tauri::Theme::Light => ResolvedTheme::Light,
        // `Theme` is non-exhaustive. An unknown variant is the same case as a query that
        // failed: stay on the light background the window was created with.
        _ => ResolvedTheme::Light,
    }
}

fn tauri_theme(theme: ResolvedTheme) -> tauri::Theme {
    match theme {
        ResolvedTheme::Dark => tauri::Theme::Dark,
        ResolvedTheme::Light => tauri::Theme::Light,
    }
}

fn chrome_color(theme: ResolvedTheme) -> tauri::webview::Color {
    // The same hex as `--background`. Light is also `tauri.conf.json`'s `backgroundColor`,
    // so a theme that could not be read does not change the colour the window was born with.
    match theme {
        ResolvedTheme::Light => tauri::webview::Color(0xF8, 0xFB, 0xFB, 0xFF),
        ResolvedTheme::Dark => tauri::webview::Color(0x0B, 0x1A, 0x1D, 0xFF),
    }
}

/// Quit routed through the close guard. macOS only: it is the only platform whose default menu
/// Tauri installs unasked, and the only one whose Quit this closes a gap in. Installing a menu
/// elsewhere would add a File > Exit that skips the guard where there was no menu at all.
#[cfg(target_os = "macos")]
mod quit_menu {
    use tauri::menu::{Menu, MenuItem, MenuItemKind};
    use tauri::{Emitter, Manager};

    use desktop::device::{CloseVerdict, DeviceManagerHandle};

    /// The Quit item's id. Not a predefined item, because a predefined one is the OS's `terminate:`.
    const QUIT_MENU_ID: &str = "quit";

    pub fn install(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
        builder.menu(app_menu).on_menu_event(|app, event| {
            if event.id() == QUIT_MENU_ID {
                request_quit(app);
            }
        })
    }

    /// Tauri's default menu, with Quit replaced by an item of our own.
    ///
    /// Cmd-Q and the application menu's Quit send `terminate:`, which tao turns straight into
    /// `RunEvent::Exit` — there is no `ExitRequested` to refuse, and `CloseRequested` never fires —
    /// so the guard that covers the window's close button was skipped (#327). An item of our own is
    /// the one place the quit can be asked before it is decided. The Dock's Quit and a logout still
    /// send `terminate:` and are not covered.
    ///
    /// The stock item is found by its label, which muda sets to `Quit {app name}` on macOS (About
    /// and Hide carry their own prefixes), because muda keeps the item's type to itself and its
    /// position is only an accident of Tauri's default layout. A miss leaves the stock Quit, so the
    /// guard fails open here as it does when the webview cannot be reached — and, like that arm,
    /// not silently: the log line is the only evidence an update to either crate disarmed it.
    fn app_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
        let menu = Menu::default(app)?;
        let mut replaced = false;
        if let Some(MenuItemKind::Submenu(application)) = menu.items()?.into_iter().next() {
            for item in application.items()? {
                let MenuItemKind::Predefined(stock) = &item else { continue };
                if !stock.text()?.starts_with("Quit ") {
                    continue;
                }
                application.remove(&item)?;
                application.append(&MenuItem::with_id(
                    app,
                    QUIT_MENU_ID,
                    format!("Quit {}", app.package_info().name),
                    true,
                    Some("CmdOrCtrl+Q"),
                )?)?;
                replaced = true;
                break;
            }
        }
        if !replaced {
            eprintln!("cuthulhu: the stock Quit item was not found, so Cmd-Q is not guarded");
        }
        // A packaged app has nowhere an operator would read that line, so a debug build, which
        // is what `cargo tauri dev` and the tests run, stops instead: an update that relabels
        // the item is found when the app is first run, not by a quit that skipped the prompt.
        debug_assert!(replaced, "the stock Quit item was not found, so Cmd-Q is not guarded");
        Ok(menu)
    }

    /// What the Quit item does: the same question the window's close button asks, and on a yes the
    /// same exit `force_quit` takes once the operator has answered the prompt.
    fn request_quit(app: &tauri::AppHandle) {
        let dev = app.state::<DeviceManagerHandle>();
        if dev.decide_close(|| app.emit("cut-in-progress", ())) == CloseVerdict::Proceed {
            app.exit(0);
        }
    }
}

/// Stops what this process is driving, shuts the device manager down and exits. Used by the UI
/// after the operator confirms they want to quit with a cut outstanding.
///
/// A Job on a Cut Host is left running: the host owns it and keeps cutting whether this desktop is
/// alive or not, while the local cutter's transport dies with this process. So what quitting stops
/// is what this process *sends* — `cancel` is cooperative, and a Silhouette has no abort command,
/// so the cutter finishes whatever moves it has already buffered either way (#158,
/// `docs/adr/0002-the-close-guard-answers-for-every-cut-this-desktop-started.md`, and
/// `apps/desktop/MANUAL-CHECKLIST.md`). The dialog the operator answered says so.
///
/// async because `shutdown` joins the local worker, and this runs on the main thread — the escape
/// hatch from the close guard hanging harder than the guard it escapes. It initiates no network
/// call of its own: the remote cancel that could block the exit is gone. What remains is a bounded
/// wait — `commit_close_after_confirmation` sets `closing`, stops local motion, then gives a send
/// already in flight up to `CLOSE_GATE_WAIT` before exiting anyway — so #116's cause is bounded
/// rather than removed. The stop sits before that wait: waiting first leaves the local worker
/// transmitting for the whole of it.
#[tauri::command(async)]
fn force_quit(app: tauri::AppHandle, dev: tauri::State<DeviceManagerHandle>) {
    dev.commit_close_after_confirmation();
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

/// Whether a page-load event invalidates the previous page's listener acknowledgement.
///
/// `Started`, not `Finished`: scripts can arm the new listener before the latter arrives, so
/// clearing on completion disarms a listener that is already live.
fn page_load_stands_down_close_guard(event: tauri::webview::PageLoadEvent) -> bool {
    matches!(event, tauri::webview::PageLoadEvent::Started)
}

fn main() {
    let (dev_handle, events) =
        DeviceManagerHandle::new(std::sync::Arc::new(HardwareBackendFactory));

    // A host that fails to load is not a reason to refuse to start — the desktop still cuts on
    // local hardware, and the operator can re-pair. Say so once rather than failing silently.
    let paired = hosts::load_or_warn(hosts::default_hosts_path().as_deref(), |e| {
        eprintln!("cuthulhu: paired hosts could not be loaded: {e}")
    });
    for host in paired {
        dev_handle.add_host(host);
    }

    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = quit_menu::install(builder);
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(AppState::new()))
        .manage(dev_handle)
        .manage(ipc::AuthorizedImages::default())
        .invoke_handler(tauri::generate_handler![
            ipc::new_doc,
            ipc::snapshot,
            ipc::commit_transform,
            ipc::commit_transforms,
            ipc::add_primitive,
            ipc::boolean_op,
            ipc::add_text,
            ipc::delete,
            ipc::reorder,
            ipc::set_cut_line_type,
            ipc::set_material_preset,
            ipc::history,
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
            ipc::cutter_statuses,
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
            ipc::get_preferences,
            ipc::set_preferences,
            set_close_guard_ready,
            force_quit,
        ])
        // A reload replaces the page but not this process, and React runs no cleanup on unload — so
        // readiness would still be `true` from the page before it, with nothing subscribed on the
        // new one. A close in that gap is refused and the warning emitted to a page that is gone:
        // no prompt, and a window the operator cannot shut. The webview's own leading stand-down
        // races the same gap because it is an async command; this one is native and precedes any
        // close the new page could see.
        //
        // `Started` only. `Finished` arrives after the page's scripts have run, so clearing there
        // would disarm a listener the new page had already installed and acknowledged — the guard
        // failing open, which is the one direction it must not fail in.
        .on_page_load(|webview, payload| {
            if page_load_stands_down_close_guard(payload.event()) {
                webview.state::<DeviceManagerHandle>().set_close_guard_ready(false);
            }
        })
        .setup(|app| {
            // The window is created hidden. Theme and the colour behind the webview are set
            // before anything can show it; the page calls `show` only after it has painted
            // the same theme. A preferences file this build will not read is left as it is
            // and the window follows the system, which is also what a missing file means.
            if let Some(window) = app.get_webview_window("main") {
                let prefs = desktop::preferences::load_or_warn(
                    desktop::preferences::default_preferences_path().as_deref(),
                    |e| eprintln!("cuthulhu: preferences could not be loaded: {e}"),
                );
                let system = window.theme().ok().map(resolved_theme);
                let chrome = desktop::preferences::launch_chrome(prefs.appearance, system);
                let theme = chrome.theme.map(tauri_theme);
                if let Err(e) = window.set_theme(theme) {
                    eprintln!("cuthulhu: window theme was not applied: {e}");
                }
                if let Err(e) = window.set_background_color(Some(chrome_color(chrome.background))) {
                    eprintln!("cuthulhu: window background was not applied: {e}");
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let dev = window.state::<DeviceManagerHandle>();
                if dev.decide_close(|| window.emit("cut-in-progress", ())) == CloseVerdict::Warn {
                    api.prevent_close();
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
                if last_progress
                    .is_some_and(|last| now.duration_since(last) < Duration::from_millis(100))
                {
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

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_page_start_stands_down_the_previous_listener() {
        assert!(super::page_load_stands_down_close_guard(
            tauri::webview::PageLoadEvent::Started
        ));
        assert!(!super::page_load_stands_down_close_guard(
            tauri::webview::PageLoadEvent::Finished
        ));
    }
}
