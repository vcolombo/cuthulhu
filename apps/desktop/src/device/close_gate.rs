// SPDX-License-Identifier: GPL-3.0-or-later
//! The barrier between a press starting a cut and a close that has been let go.
//!
//! One type rather than a lock and a flag side by side, because the three obligations are only
//! correct together: take the gate, check the flag, and keep holding it across whatever starts the
//! cut. As loose statements at a call site, each was one edit away from being dropped — and the
//! order inside `commit_after_confirmation` (set the flag, *then* wait) was one swap away from
//! making its own wait the window it exists to close.
//!
//! The mark a remote dispatch writes lives inside the host connection, so a press queued behind a
//! poll has written nothing and no mark can answer for it; a local press is invisible between the
//! aim check and the worker. Both are a Job about to start, and both must not start into a process
//! on its way out (#158).

/// The barrier itself: private fields, so `permit` is the only way to hold one.
///
/// A module rather than two fields on the handle, because the obligations are only correct
/// together and the compiler is what has to keep them that way. Reachable from the parent
/// module, a permit could be built straight from the lock — blocking a close, which looks
/// right, while never reading the flag, which is the whole check.
#[derive(Default)]
pub(super) struct CloseGate {
    sends: std::sync::RwLock<()>,
    closing: std::sync::atomic::AtomicBool,
}

/// Permission to start a cut, held until the request is away.
#[must_use = "the permit must be held across whatever starts the cut"]
pub(super) struct SendPermit<'a>(#[allow(dead_code)] std::sync::RwLockReadGuard<'a, ()>);

impl CloseGate {
    /// Permission for one press, or `None` if the window has already gone.
    pub(super) fn permit(&self) -> Option<SendPermit<'_>> {
        let sends = self.sends.read().unwrap();
        if self.closing.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        Some(SendPermit(sends))
    }

    /// Commit to a close if `idle` agrees nothing is outstanding, answering and committing as
    /// one step so a press cannot cross into a send in between.
    ///
    /// `try_write` rather than `write`: a send in flight is a warning owed, not something for
    /// the main thread to wait out.
    pub(super) fn commit_if(&self, idle: impl FnOnce() -> bool) -> bool {
        let Ok(_sends) = self.sends.try_write() else { return false };
        if !idle() {
            return false;
        }
        self.closing.store(true, std::sync::atomic::Ordering::SeqCst);
        true
    }

    /// Commit to a close that is happening whatever is outstanding — the two stand-downs, where
    /// the window goes because the webview could not be told.
    ///
    /// Nothing is waited for, because this runs on the main thread inside the close callback,
    /// where waiting is the freeze the guard exists to avoid (#115). So it stops every press
    /// that has yet to ask and no others: a permit already out will finish its send, exactly as
    /// one that outlasts the confirmed quit's wait does.
    pub(super) fn commit_regardless(&self) {
        self.closing.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Commit once the operator has answered the prompt.
    ///
    /// Set first, waited second: with the flag already set no press that has yet to ask can
    /// send, and the wait is only for one already in progress. Bounded, because "quit anyway"
    /// must not hang behind a Pi that is timing out — a send that outlasts it may still land,
    /// which is what the prompt says about a Job sent to a Cut Host.
    pub(super) fn commit_after_confirmation(&self) {
        self.commit_regardless();
        let deadline = std::time::Instant::now() + super::CLOSE_GATE_WAIT;
        while self.sends.try_write().is_err() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Whether a close has been committed. For the test that pins the commit-before-wait order;
    /// production asks by taking a permit.
    #[cfg(test)]
    pub(super) fn is_closing(&self) -> bool {
        self.closing.load(std::sync::atomic::Ordering::SeqCst)
    }
}
