// SPDX-License-Identifier: GPL-3.0-or-later
//! The reply the OS is owed after a deferred terminate (#336).
//!
//! One type rather than a flag on the handle, for the reason `CloseGate` is one: the obligations
//! are only correct together. The debt is claimed *before* the decision can raise a prompt, because
//! the operator's answer arrives on another thread and may land before the decision returns; it is
//! cleared again when nothing was asked, because nobody will answer; a second request while it is
//! held joins it rather than raising a second prompt; and `take` is the single swap that hands it
//! to whichever of "Quit anyway", "Cancel" or a replaced page gets there first. As loose statements
//! on the handle, each was one edit away from being dropped, and the OS is left waiting — or told
//! twice — when one is.
//!
//! Platform-independent on purpose: the macOS adapter (`terminate.rs`) only maps the answer onto
//! the NS constants, so the protocol is exercised by tests on every platform.

use super::{CloseVerdict, TerminateAnswer};

/// Whether a macOS terminate has been deferred (`NSTerminateLater`) and the OS is still owed its
/// reply. Not a fact about presses, so not part of `CloseGate`: it is about a question asked of
/// the OS, and only an answer to that question clears it.
///
/// Private field, so `answer` and `take` are the only ways to move it.
#[derive(Default)]
pub(super) struct TerminateReply {
    owed: std::sync::atomic::AtomicBool,
}

impl TerminateReply {
    /// Answer a request to terminate with `decide`, and record whether a reply is now owed.
    ///
    /// `decide` is the one decision every door shares (`decide_close`); it is not called for a
    /// request that joins a deferral already pending. `Warn` is `Later` and leaves the debt
    /// standing, `Proceed` is `Now` and clears it, because the OS is told to go and nobody will
    /// answer a prompt that was never raised.
    pub(super) fn answer(&self, decide: impl FnOnce() -> CloseVerdict) -> TerminateAnswer {
        if self.owed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return TerminateAnswer::Later;
        }
        match decide() {
            CloseVerdict::Warn => TerminateAnswer::Later,
            CloseVerdict::Proceed => {
                self.owed.store(false, std::sync::atomic::Ordering::SeqCst);
                TerminateAnswer::Now
            }
        }
    }

    /// Whether a reply was owed, clearing it. `true` at most once per deferral, so a confirm, a
    /// cancel and a page that went away cannot between them reply twice. `false` for every other
    /// door: a window close or the Quit item has nothing pending to answer.
    pub(super) fn take(&self) -> bool {
        self.owed.swap(false, std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CloseVerdict::{Proceed, Warn};
    use TerminateAnswer::{Later, Now};

    /// Each arm is a different promise to the OS, and a reply sent twice or never is a logout that
    /// hangs or an app that quits on a cancel.
    #[test]
    fn a_reply_is_owed_only_for_a_deferral() {
        let reply = TerminateReply::default();
        assert_eq!(reply.answer(|| Proceed), Now);
        assert!(!reply.take(), "the OS was told to go, so nobody owes it a reply");

        assert_eq!(reply.answer(|| Warn), Later);
        assert!(reply.take(), "a deferral owes the OS a reply");
        assert!(!reply.take(), "the reply was owed twice: a confirm and a cancel could both send it");
    }

    /// The answer is emitted to a webview that answers on another thread, so it can land before
    /// the decision returns. The debt is owed *inside* `decide`: marking it afterwards, on `Warn`,
    /// would let that answer find nothing owed and leave the deferral it answered outstanding.
    #[test]
    fn the_debt_is_owed_while_the_prompt_is_still_being_raised() {
        let reply = TerminateReply::default();
        let answer = reply.answer(|| {
            assert!(reply.take(), "an answer during the warning found no reply owed");
            Warn
        });
        assert_eq!(answer, Later);
        assert!(!reply.take(), "that early answer did not settle the reply");

        assert_eq!(reply.answer(|| Warn), Later);
        assert!(reply.take(), "a deferral after an early answer owes no reply");
    }

    /// A prompt that is already up is joined, not repeated, and the join does not make a second
    /// reply owed. Once answered, the next request is decided afresh.
    #[test]
    fn a_second_request_joins_the_pending_deferral() {
        let asked = std::cell::Cell::new(0);
        let decide = || {
            asked.set(asked.get() + 1);
            Warn
        };
        let reply = TerminateReply::default();
        assert_eq!(reply.answer(decide), Later);
        assert_eq!(reply.answer(decide), Later);
        assert_eq!(asked.get(), 1, "a second request raised a second prompt over the first");
        assert!(reply.take());
        assert!(!reply.take());

        assert_eq!(reply.answer(decide), Later);
        assert_eq!(asked.get(), 2, "an answered deferral was joined instead of decided afresh");
        assert!(reply.take());
    }
}
