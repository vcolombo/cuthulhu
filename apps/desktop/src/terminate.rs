// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS only: answer `applicationShouldTerminate:` for the Dock's Quit and for a logout, restart
//! or shutdown (#336).
//!
//! Those paths send `terminate:` straight to `NSApp`. tao's delegate implements only
//! `applicationWillTerminate:`, which becomes `RunEvent::Exit` with no `ExitRequested` before it,
//! and tauri-apps/tauri#12978 asks upstream for the hook and is open. So the method is added to
//! the delegate class tao has already registered, through the Objective-C runtime. That is the
//! reason this module has `unsafe` and a direct `objc2` dependency, and the reason it is the only
//! place that does: the decision itself is `DeviceManagerHandle::answer_terminate`, not a copy.

use std::ffi::CStr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use desktop::device::TerminateAnswer;
use objc2::ffi::class_addMethod;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{msg_send, sel};

/// The class tao registers as the `NSApplication` delegate
/// [src: tao 0.35.3 src/platform_impl/macos/app_delegate.rs L47-L90 (Apache-2.0)].
const TAO_DELEGATE_CLASS: &CStr = c"TaoAppDelegateParent";

/// `NSApplicationTerminateReply`
/// [doc: NSApplication.TerminateReply, https://developer.apple.com/documentation/appkit/nsapplication/terminatereply].
/// `NSTerminateCancel` (0) is never returned: a cancel is the operator's answer, and arrives
/// through `reply`.
const NS_TERMINATE_NOW: usize = 1;
const NS_TERMINATE_LATER: usize = 2;

type Decide = Box<dyn Fn() -> TerminateAnswer + Send + Sync>;

/// A method implementation is a bare function pointer with nowhere to keep this, and the OS calls
/// it once per process: a static is the honest shape.
static DECIDE: OnceLock<Decide> = OnceLock::new();

extern "C" fn application_should_terminate(_this: &AnyObject, _cmd: Sel, _sender: *mut AnyObject) -> usize {
    // A panic must not unwind into AppKit. The guard failing open is the direction every other arm
    // of `decide_close` already takes when it cannot ask.
    let answer = catch_unwind(AssertUnwindSafe(|| DECIDE.get().map(|decide| decide())));
    match answer {
        Ok(Some(TerminateAnswer::Later)) => NS_TERMINATE_LATER,
        Ok(Some(TerminateAnswer::Now)) | Ok(None) | Err(_) => NS_TERMINATE_NOW,
    }
}

/// Make the app delegate ask `decide` before the OS terminates the app. A failure is logged and
/// leaves stock behaviour in place, which fails open like the rest of the close guard.
pub fn install(decide: impl Fn() -> TerminateAnswer + Send + Sync + 'static) {
    if !install_on(TAO_DELEGATE_CLASS, decide) {
        eprintln!("cuthulhu: the Dock's Quit and a logout are not guarded: applicationShouldTerminate: was not installed");
    }
}

/// Whether the method was added. `class_addMethod` refuses a class that already implements the
/// selector, so a future tao that does would show here as a miss, not as two handlers.
fn install_on(class_name: &CStr, decide: impl Fn() -> TerminateAnswer + Send + Sync + 'static) -> bool {
    let Some(class) = AnyClass::get(class_name) else {
        eprintln!("cuthulhu: no app delegate class named {class_name:?}");
        return false;
    };
    // Safety: `class_addMethod` takes the runtime's own lock, and this runs during startup before
    // the delegate is asked anything. The implementation has the signature the type encoding
    // `Q@:@` promises: `NSUInteger` out, `self`, `_cmd` and one object in. The cast to `Imp`
    // changes only the unwind ABI string, and the function never unwinds.
    let added = unsafe {
        let imp = std::mem::transmute::<extern "C" fn(&AnyObject, Sel, *mut AnyObject) -> usize, Imp>(
            application_should_terminate,
        );
        class_addMethod(class as *const AnyClass as *mut AnyClass, sel!(applicationShouldTerminate:), imp, c"Q@:@".as_ptr())
    };
    if !added.as_bool() {
        return false;
    }
    DECIDE.set(Box::new(decide)).is_ok()
}

/// Tell the OS what the operator decided about a terminate this module deferred. `proceed: true`
/// lets the app go; `false` cancels the logout or quit and the app stays open.
///
/// `replyToApplicationShouldTerminate:` belongs on the main thread, and the commands that call
/// this run on Tauri's pool.
pub fn reply(app: &tauri::AppHandle, proceed: bool) {
    let queued = app.run_on_main_thread(move || {
        let Some(class) = AnyClass::get(c"NSApplication") else { return };
        // Safety: both selectors exist on `NSApplication`, and `sharedApplication` returns the
        // process-wide instance, which outlives this call.
        unsafe {
            let ns_app: *mut AnyObject = msg_send![class, sharedApplication];
            let _: () = msg_send![ns_app, replyToApplicationShouldTerminate: Bool::new(proceed)];
        }
    });
    if let Err(e) = queued {
        eprintln!("cuthulhu: the OS was not told whether to terminate, and may be left waiting: {e}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use objc2::runtime::ClassBuilder;

    use super::*;

    static DEFER: AtomicBool = AtomicBool::new(false);

    fn ask(class: &AnyClass) -> usize {
        // Safety: `class` is a plain `NSObject` subclass this test registered, and the selector is
        // the one `install_on` just added to it.
        unsafe {
            let object: *mut AnyObject = msg_send![class, new];
            msg_send![object, applicationShouldTerminate: std::ptr::null_mut::<AnyObject>()]
        }
    }

    /// The runtime-added method answers what the decider says, and is not added over a class that
    /// already has one. A real delegate cannot be built here, so a class of the same shape stands in.
    #[test]
    fn the_added_method_returns_the_deciders_answer_and_never_overrides_one() {
        let nsobject = AnyClass::get(c"NSObject").unwrap();
        let fresh = ClassBuilder::new(c"CuthulhuTerminateTestDelegate", nsobject).unwrap().register();

        assert!(install_on(c"CuthulhuTerminateTestDelegate", || {
            if DEFER.load(Ordering::SeqCst) { TerminateAnswer::Later } else { TerminateAnswer::Now }
        }));
        DEFER.store(false, Ordering::SeqCst);
        assert_eq!(ask(fresh), NS_TERMINATE_NOW);
        DEFER.store(true, Ordering::SeqCst);
        assert_eq!(ask(fresh), NS_TERMINATE_LATER, "a deferral must come back as NSTerminateLater");

        extern "C" fn theirs(_: &AnyObject, _: Sel, _: *mut AnyObject) -> usize {
            0
        }
        let mut occupied = ClassBuilder::new(c"CuthulhuTerminateTestOccupied", nsobject).unwrap();
        unsafe { occupied.add_method(sel!(applicationShouldTerminate:), theirs as extern "C" fn(_, _, _) -> _) };
        let occupied = occupied.register();
        assert!(!install_on(c"CuthulhuTerminateTestOccupied", || TerminateAnswer::Now));
        assert_eq!(ask(occupied), 0, "a method the class already had was replaced");

        assert!(!install_on(c"CuthulhuNoSuchDelegate", || TerminateAnswer::Now));
    }
}
