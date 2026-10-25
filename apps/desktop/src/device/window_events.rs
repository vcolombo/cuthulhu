// SPDX-License-Identifier: GPL-3.0-or-later
//! The two native hooks the close guard hangs on. They are generic over the runtime and live here,
//! not inline in `main`'s builder, so a test can register the very functions the app registers on
//! Tauri's mock runtime (#326): inline closures were reachable from nothing but a real window.

use tauri::webview::{PageLoadEvent, PageLoadPayload};
use tauri::{Emitter, Manager, Runtime, Webview, Window, WindowEvent};

use super::{CloseVerdict, DeviceManagerHandle};

/// The window's close button, and anything else that asks a window to close.
///
/// The verdict is `decide_close`'s, shared with the Quit item; this only turns a `Warn` into a
/// refusal, because the window staying open is what gives the operator's answer somewhere to land.
pub fn on_close_requested<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        let dev = window.state::<DeviceManagerHandle>();
        if dev.decide_close(|| window.emit("cut-in-progress", ())) == CloseVerdict::Warn {
            api.prevent_close();
        }
    }
}

/// A reload replaces the page but not this process, and React runs no cleanup on unload — so
/// readiness would still be `true` from the page before it, with nothing subscribed on the new
/// one. A close in that gap is refused and the warning emitted to a page that is gone: no prompt,
/// and a window the operator cannot shut. The webview's own leading stand-down races the same gap
/// because it is an async command; this one is native and precedes any close the new page could
/// see.
pub fn on_page_load<R: Runtime>(webview: &Webview<R>, payload: &PageLoadPayload<'_>) {
    note_page_load(&webview.state::<DeviceManagerHandle>(), payload.event());
}

/// `Started` only. `Finished` arrives after the page's scripts have run, so clearing there would
/// disarm a listener the new page had already installed and acknowledged — the guard failing
/// open, which is the one direction it must not fail in.
///
/// Split from `on_page_load` because `PageLoadPayload` cannot be built outside Tauri.
pub(super) fn note_page_load(dev: &DeviceManagerHandle, event: PageLoadEvent) {
    if matches!(event, PageLoadEvent::Started) {
        dev.set_close_guard_ready(false);
    }
}
